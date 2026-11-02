//! **The liveness report** (nxf 6j6v.t7xp; design 6j6v.pszd section 6.4 as amended by its
//! dispositions) — `nxc liveness` and its seam [`crate::engine::Engine::liveness`], over
//! [`liveness`].
//!
//! The report reads nothing but the ladder's own record, `recovery_log`: the policy pair every row
//! carries (cut 5, 6j6v.0z5x), and the episode the ladder itself delimits by the rows it writes.
//! It decides nothing and writes nothing — it is the owner's comparison of the policies A, B and C
//! over the evidence the trial collected.
//!
//! Re-exported from [`crate::facade`]; the module itself is private, so the public path is
//! `facade::liveness` and `facade::episodes_of`.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::error::{NxfError, Result};
use crate::facade::Reader;
use crate::ladder::RecoveryLogRow;
use crate::store::ChatStore;
use crate::workspace::{LivenessPolicy, ResolvedLiveness};

/// The rows that OPEN an episode — when they follow an end row, or are the slot's first row.
const OPENERS: [&str; 4] = ["asked", "stopped", "continued", "recommissioned"];
/// The outcomes of a rung the ladder took back — nothing happened to the session.
const REVERTS: [&str; 3] = ["already_running", "no_process", "cannot_ask"];
/// The events the report counts, each always present in [`WorkspaceLiveness::counts`].
const COUNTED: [&str; 10] = [
    "asked",
    "confirmed",
    "recovered",
    "stopped",
    "continued",
    "recommissioned",
    "noticed",
    "answered",
    "escalated",
    "withdrawn",
];
/// The window a report covers when none is given: the last seven days up to `now`.
const DEFAULT_WINDOW_DAYS: i64 = 7;

/// The instants a report covers: rows with `from <= at < to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessWindow<'a> {
    pub from: &'a str,
    pub to: &'a str,
}

/// Which workspace a block is about — its name and its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LivenessWorkspace {
    pub name: String,
    pub path: String,
}

impl LivenessWorkspace {
    /// The identity of a resolved workspace: its root, and the root's last path component as its
    /// name. The registry's name is used instead where there is one (`--everywhere`).
    pub fn of(ws: &crate::workspace::Workspace) -> Result<LivenessWorkspace> {
        use crate::workspace::ChatWorkspaceExt;
        let root = ws.workspace_root()?;
        Ok(LivenessWorkspace {
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: root.to_string_lossy().into_owned(),
        })
    }
}

/// How an episode ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeEnd {
    /// The session showed life.
    Recovered,
    /// It replied while the episode was open.
    Answered,
    /// The ladder handed it up.
    Escalated,
    /// The thread stopped owing the answer — or the ladder took the episode's only rung back.
    Withdrawn,
    /// No end row yet.
    Open,
}

impl EpisodeEnd {
    /// The word the report uses for it — the same as its `--json` value.
    pub fn as_str(self) -> &'static str {
        match self {
            EpisodeEnd::Recovered => "recovered",
            EpisodeEnd::Answered => "answered",
            EpisodeEnd::Escalated => "escalated",
            EpisodeEnd::Withdrawn => "withdrawn",
            EpisodeEnd::Open => "open",
        }
    }
}

/// What became of one resume (`continued` / `recommissioned`), read from the slot's next row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeOutcome {
    /// Next row `recovered`.
    Recovered,
    /// Next row `answered`.
    Answered,
    /// Next row `stopped` — or another resume: the session was dead again.
    DeadAgain,
    /// Next row `escalated`.
    Escalated,
    /// Next row `asked`.
    NudgedAgain,
    /// Next row `withdrawn`: the thread stopped owing the answer before the resume showed anything.
    Withdrawn,
    /// No next row yet.
    Pending,
}

impl ResumeOutcome {
    /// The word the report uses for it — the same as its `--json` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeOutcome::Recovered => "recovered",
            ResumeOutcome::Answered => "answered",
            ResumeOutcome::DeadAgain => "dead_again",
            ResumeOutcome::Escalated => "escalated",
            ResumeOutcome::NudgedAgain => "nudged_again",
            ResumeOutcome::Withdrawn => "withdrawn",
            ResumeOutcome::Pending => "pending",
        }
    }
}

/// **One episode** of one slot: from an opener to its end row, inclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Episode {
    pub thread: String,
    pub role: String,
    /// `(dead_resumes, notice_at_first_stop)` of the episode's first row; `None` on a row older
    /// than the policy columns.
    pub policy: Option<(u32, bool)>,
    pub letter: Option<char>,
    pub silent_since: Option<String>,
    /// The `at` of the episode's first `asked` row.
    pub first_nudge: Option<String>,
    /// Every resume, as `(attempt, outcome)`.
    pub resumes: Vec<(u32, ResumeOutcome)>,
    pub ended: EpisodeEnd,
    /// The `attempt` of the end row.
    pub end_attempt: Option<u32>,
    /// `silent_since` of the first row to the `at` of the `recovered` or `answered` end row.
    pub silence_to_recovery_secs: Option<i64>,
    /// `silent_since` of the first row to the `at` of the `escalated` end row.
    pub silence_to_escalation_secs: Option<i64>,
    /// The log row ids, for a reader who wants to look.
    pub rows: Vec<i64>,
}

/// **What the owner compares**, per policy pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicySummary {
    pub policy: (u32, bool),
    pub letter: Option<char>,
    pub episodes: u32,
    /// The middle value; for an even count, the mean of the two middle values, rounded down.
    pub median_silence_to_recovery_secs: Option<i64>,
    pub max_silence_to_recovery_secs: Option<i64>,
    pub median_silence_to_escalation_secs: Option<i64>,
    pub max_silence_to_escalation_secs: Option<i64>,
    /// How many episodes resume 1, 2 and 3 rescued — a resume whose next row is `recovered` or
    /// `answered`.
    pub rescued_by_resume: [u32; 3],
    pub escalated: u32,
}

/// **One workspace's block** of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceLiveness {
    pub workspace: LivenessWorkspace,
    /// The policy in force NOW, as the workspace's config reads.
    pub policy_now: LivenessPolicy,
    pub letter_now: Option<char>,
    /// What reading `[chat.liveness]` found wrong.
    pub config_warnings: Vec<String>,
    /// Unknown keys under `[chat.liveness]`, verbatim.
    pub unknown_keys: Vec<String>,
    /// The policy pairs the window's rows carry, each with its letter where it has one.
    pub policies_in_rows: Vec<((u32, bool), Option<char>)>,
    /// Rows per event in the window — every counted event present, `0` included.
    pub counts: BTreeMap<String, u32>,
    /// The episodes whose first row lies in the window, oldest slot first.
    pub episodes: Vec<Episode>,
    pub summary: Vec<PolicySummary>,
}

/// A workspace the report read, or one it could not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum WorkspaceBlock {
    Read(WorkspaceLiveness),
    Unreadable {
        workspace: LivenessWorkspace,
        error: String,
    },
}

/// **The report**: the window and one block per workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LivenessReport {
    pub from: String,
    pub to: String,
    pub workspaces: Vec<WorkspaceBlock>,
}

/// **Who may read the report: the board's owner, nobody else** (6j6v.pszd round 3 CQ #3 / IR #6).
/// It exposes every operation's recovery history, so a handle — an agent — is refused as well as
/// an account that is not the owner (whom [`Reader`]'s one rule refuses already). A handle is
/// admitted by that rule for the conversation reads, so this asks it for the reader's standing and
/// refuses every standing that is a chat identity.
pub fn admit_liveness_reader(reader: Reader<'_>) -> Result<()> {
    match reader.standing()?.handle() {
        None => Ok(()),
        Some(_) => Err(NxfError::forbidden(
            "only the board's owner reads the liveness report",
        )),
    }
}

/// The default window's start: seven days before `now`.
pub fn default_liveness_from(now: &str) -> Result<String> {
    crate::facade::instant_after(
        now,
        -time::Duration::days(DEFAULT_WINDOW_DAYS),
        "the liveness window",
    )
}

/// **One workspace's block** over `store`: the counts of the window's rows, the episodes whose
/// first row lies in the window, and the summary per policy pair. The workspace's identity and its
/// policy are ARGUMENTS (round 2 CQ #5): the store does not know which config it belongs to.
pub fn liveness(
    store: &ChatStore,
    window: LivenessWindow<'_>,
    workspace: &LivenessWorkspace,
    config: &ResolvedLiveness,
) -> Result<WorkspaceLiveness> {
    for (flag, value) in [("--from", window.from), ("--to", window.to)] {
        parse(value).ok_or_else(|| {
            NxfError::validation(format!("{flag} `{value}` is not an RFC3339 instant"))
        })?;
    }
    let in_window = |at: &str| {
        !crate::stranded::earlier(at, window.from) && crate::stranded::earlier(at, window.to)
    };
    let rows = store.recovery_log_between(window.from, window.to)?;

    let mut counts: BTreeMap<String, u32> = COUNTED.iter().map(|e| (e.to_string(), 0)).collect();
    let mut pairs: Vec<(u32, bool)> = Vec::new();
    let mut slots: Vec<(String, String)> = Vec::new();
    for row in &rows {
        if let Some(n) = counts.get_mut(row.action.as_str()) {
            *n += 1;
        }
        if let Some(pair) = pair_of(row) {
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
        let slot = (row.thread.clone(), row.role.clone());
        if !slots.contains(&slot) {
            slots.push(slot);
        }
    }
    pairs.sort();

    // An episode is read from the WHOLE slot — the rows before the window decide whether its
    // first row follows an end row, the rows after it how it ended.
    let mut episodes = Vec::new();
    for (thread, role) in &slots {
        let slot_rows = store.slot_recovery_log(thread, role)?;
        episodes.extend(
            episodes_of(&slot_rows)
                .into_iter()
                .filter(|e| first_at(&slot_rows, e).is_some_and(in_window)),
        );
    }

    Ok(WorkspaceLiveness {
        workspace: workspace.clone(),
        policy_now: config.policy,
        letter_now: config.policy.letter(),
        config_warnings: config.warnings.clone(),
        unknown_keys: config.unknown_keys.clone(),
        policies_in_rows: pairs.iter().map(|p| (*p, letter_of(*p))).collect(),
        counts,
        summary: summarize(&episodes),
        episodes,
    })
}

/// **The episodes of a log** (design 6.3 / 6.4 as amended): grouped by slot, each walked oldest
/// first.
///
/// - An episode starts at an opener (`asked`, `stopped`, `continued`, `recommissioned`) that is the
///   slot's first row or follows an end row; rows that are neither opener nor end (`confirmed`,
///   `noticed`) belong to the open episode, and outside one they are passed over.
/// - It ends at, and includes, the next end row (`recovered`, `answered`, `escalated`,
///   `withdrawn`); `open` while none has come.
/// - An opener with a revert outcome that is the episode's only row, and that no row of its own
///   episode follows — the slot ends there, or its next row is another opener — is an episode
///   `withdrawn`: the ladder took the attempt back, so what comes next starts afresh.
/// - A resume's outcome is the slot's next row (a `confirmed` or `noticed` row passed over); a
///   resume with a revert outcome started nothing and is not listed.
pub fn episodes_of(rows: &[RecoveryLogRow]) -> Vec<Episode> {
    let mut sorted: Vec<&RecoveryLogRow> = rows.iter().collect();
    sorted.sort_by(|a, b| (&a.thread, &a.role, a.id).cmp(&(&b.thread, &b.role, b.id)));

    let mut out = Vec::new();
    for slot in sorted.chunk_by(|a, b| a.thread == b.thread && a.role == b.role) {
        let mut open: Option<Episode> = None;
        for (i, row) in slot.iter().enumerate() {
            let action = row.action.as_str();
            let ep = match open.as_mut() {
                Some(ep) => ep,
                None if OPENERS.contains(&action) => open.insert(start(row)),
                None => continue,
            };
            if !ep.rows.contains(&row.id) {
                ep.rows.push(row.id);
            }
            if action == "asked" && ep.first_nudge.is_none() {
                ep.first_nudge = Some(row.at.clone());
            }
            let reverted = row.outcome.as_deref().is_some_and(|o| REVERTS.contains(&o));
            // A resume the ladder took back started nothing, so it is no resume.
            if (action == "continued" || action == "recommissioned") && !reverted {
                ep.resumes
                    .push((row.attempt, resume_outcome(next_rung(&slot[i + 1..]))));
            }
            if let Some(end) = end_of(action) {
                let ep = open.take().expect("an episode is open");
                out.push(finish(ep, row, end));
                continue;
            }
            let followed = slot
                .get(i + 1)
                .is_some_and(|n| !OPENERS.contains(&n.action.as_str()));
            if ep.rows.len() == 1 && reverted && !followed {
                let mut ep = open.take().expect("an episode is open");
                ep.ended = EpisodeEnd::Withdrawn;
                ep.end_attempt = Some(row.attempt);
                out.push(ep);
            }
        }
        out.extend(open);
    }
    out
}

fn start(row: &RecoveryLogRow) -> Episode {
    let policy = pair_of(row);
    Episode {
        thread: row.thread.clone(),
        role: row.role.clone(),
        policy,
        letter: policy.and_then(letter_of),
        silent_since: row.silent_since.clone(),
        first_nudge: None,
        resumes: Vec::new(),
        ended: EpisodeEnd::Open,
        end_attempt: None,
        silence_to_recovery_secs: None,
        silence_to_escalation_secs: None,
        rows: vec![row.id],
    }
}

fn finish(mut ep: Episode, end_row: &RecoveryLogRow, end: EpisodeEnd) -> Episode {
    ep.ended = end;
    ep.end_attempt = Some(end_row.attempt);
    let span = ep
        .silent_since
        .as_deref()
        .and_then(|s| seconds_between(s, &end_row.at));
    match end {
        EpisodeEnd::Recovered | EpisodeEnd::Answered => ep.silence_to_recovery_secs = span,
        EpisodeEnd::Escalated => ep.silence_to_escalation_secs = span,
        EpisodeEnd::Withdrawn | EpisodeEnd::Open => {}
    }
    ep
}

/// The four rows that END an episode, and are its last row.
fn end_of(action: &str) -> Option<EpisodeEnd> {
    Some(match action {
        "recovered" => EpisodeEnd::Recovered,
        "answered" => EpisodeEnd::Answered,
        "escalated" => EpisodeEnd::Escalated,
        "withdrawn" => EpisodeEnd::Withdrawn,
        _ => return None,
    })
}

/// The slot's next row that ACTS — a `confirmed` or `noticed` row says nothing about whether the
/// resume brought the session back.
fn next_rung<'r>(after: &[&'r RecoveryLogRow]) -> Option<&'r RecoveryLogRow> {
    after
        .iter()
        .copied()
        .find(|r| !matches!(r.action.as_str(), "confirmed" | "noticed" | "progress"))
}

fn resume_outcome(next: Option<&RecoveryLogRow>) -> ResumeOutcome {
    match next.map(|r| r.action.as_str()) {
        None => ResumeOutcome::Pending,
        Some("recovered") => ResumeOutcome::Recovered,
        Some("answered") => ResumeOutcome::Answered,
        Some("escalated") => ResumeOutcome::Escalated,
        Some("asked") => ResumeOutcome::NudgedAgain,
        Some("withdrawn") => ResumeOutcome::Withdrawn,
        // `stopped`, or another resume: the session was dead again.
        Some(_) => ResumeOutcome::DeadAgain,
    }
}

fn pair_of(row: &RecoveryLogRow) -> Option<(u32, bool)> {
    Some((row.policy_dead_resumes?, row.policy_notice?))
}

fn letter_of((dead_resumes, notice_at_first_stop): (u32, bool)) -> Option<char> {
    LivenessPolicy {
        dead_resumes,
        notice_at_first_stop,
        fell_back: false,
    }
    .letter()
}

fn first_at<'r>(slot: &'r [RecoveryLogRow], ep: &Episode) -> Option<&'r str> {
    let first = *ep.rows.first()?;
    slot.iter().find(|r| r.id == first).map(|r| r.at.as_str())
}

fn summarize(episodes: &[Episode]) -> Vec<PolicySummary> {
    let mut by_pair: BTreeMap<(u32, bool), Vec<&Episode>> = BTreeMap::new();
    for ep in episodes {
        if let Some(pair) = ep.policy {
            by_pair.entry(pair).or_default().push(ep);
        }
    }
    by_pair
        .into_iter()
        .map(|(pair, eps)| {
            let recovery: Vec<i64> = eps
                .iter()
                .filter_map(|e| e.silence_to_recovery_secs)
                .collect();
            let escalation: Vec<i64> = eps
                .iter()
                .filter_map(|e| e.silence_to_escalation_secs)
                .collect();
            let mut rescued = [0u32; 3];
            for (n, outcome) in eps.iter().flat_map(|e| e.resumes.iter()) {
                let rescue = matches!(outcome, ResumeOutcome::Recovered | ResumeOutcome::Answered);
                if rescue && (1..=3).contains(n) {
                    rescued[(*n - 1) as usize] += 1;
                }
            }
            PolicySummary {
                policy: pair,
                letter: letter_of(pair),
                episodes: eps.len() as u32,
                median_silence_to_recovery_secs: median(&recovery),
                max_silence_to_recovery_secs: recovery.iter().copied().max(),
                median_silence_to_escalation_secs: median(&escalation),
                max_silence_to_escalation_secs: escalation.iter().copied().max(),
                rescued_by_resume: rescued,
                escalated: eps
                    .iter()
                    .filter(|e| e.ended == EpisodeEnd::Escalated)
                    .count() as u32,
            }
        })
        .collect()
}

fn median(values: &[i64]) -> Option<i64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]).div_euclid(2)),
    }
}

fn parse(at: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).ok()
}

fn seconds_between(a: &str, b: &str) -> Option<i64> {
    Some((parse(b)? - parse(a)?).whole_seconds())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, at: &str, action: &str, attempt: u32, outcome: &str) -> RecoveryLogRow {
        RecoveryLogRow {
            id,
            thread: "t".into(),
            role: "ws/coder".into(),
            at: at.into(),
            action: action.into(),
            attempt,
            session: Some("s".into()),
            outcome: Some(outcome.into()),
            detail: String::new(),
            retries: 0,
            policy_dead_resumes: Some(1),
            policy_notice: Some(false),
            silent_since: Some("2026-10-24T10:00:00Z".into()),
            rest_secs: None,
        }
    }

    #[test]
    fn an_overtaken_nudge_ending_in_recovered_is_closed_and_a_later_nudge_is_a_second_episode() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "overtaken"),
            row(2, "2026-10-24T10:20:00Z", "recovered", 0, "life"),
            row(3, "2026-10-24T10:50:00Z", "asked", 0, "asked"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(eps.len(), 2);
        assert_eq!(
            (eps[0].ended, eps[1].ended),
            (EpisodeEnd::Recovered, EpisodeEnd::Open)
        );
        assert_eq!(eps[0].rows, vec![1, 2]);
        assert_eq!(eps[0].silence_to_recovery_secs, Some(1200));
    }

    #[test]
    fn b_recovers_after_resume_one_then_escalates_after_one_resume_never_a_second() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:15:00Z", "stopped", 0, "stop_requested"),
            row(3, "2026-10-24T10:17:00Z", "continued", 1, "triggered"),
            row(4, "2026-10-24T10:19:00Z", "recovered", 1, "life"),
            row(5, "2026-10-24T10:28:00Z", "asked", 1, "asked"),
            row(6, "2026-10-24T10:33:00Z", "stopped", 1, "stop_requested"),
            row(7, "2026-10-24T10:33:00Z", "escalated", 1, "escalated"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].resumes, vec![(1, ResumeOutcome::Recovered)]);
        assert_eq!(eps[0].ended, EpisodeEnd::Recovered);
        assert_eq!(
            (eps[1].ended, eps[1].end_attempt, eps[1].resumes.len()),
            (EpisodeEnd::Escalated, Some(1), 0)
        );
        assert_eq!(eps[1].first_nudge.as_deref(), Some("2026-10-24T10:28:00Z"));
    }

    #[test]
    fn confirm_then_recovered_is_one_episode_ended_by_life() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:12:00Z", "confirmed", 0, "granted"),
            row(3, "2026-10-24T10:37:00Z", "recovered", 0, "life"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!((eps.len(), eps[0].ended), (1, EpisodeEnd::Recovered));
        assert_eq!(eps[0].rows, vec![1, 2, 3]);
    }

    #[test]
    fn an_ordinary_recovery_opens_an_episode_too() {
        let rows = [
            row(1, "2026-10-24T10:05:00Z", "continued", 1, "triggered"),
            row(2, "2026-10-24T10:08:00Z", "recovered", 1, "life"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(
            (eps.len(), eps[0].ended, eps[0].resumes.clone()),
            (
                1,
                EpisodeEnd::Recovered,
                vec![(1, ResumeOutcome::Recovered)]
            )
        );
    }

    #[test]
    fn a_reverted_opener_is_an_episode_withdrawn_and_a_nudge_after_it_is_a_new_one() {
        let rows = [
            row(1, "2026-10-24T10:05:00Z", "continued", 0, "already_running"),
            row(2, "2026-10-24T10:20:00Z", "asked", 0, "asked"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(
            eps.iter().map(|e| e.ended).collect::<Vec<_>>(),
            vec![EpisodeEnd::Withdrawn, EpisodeEnd::Open]
        );
        assert!(eps[0].resumes.is_empty(), "{:?}", eps[0].resumes);
    }

    #[test]
    fn a_reverted_opener_that_ends_the_slot_is_withdrawn_too() {
        let rows = [row(1, "2026-10-24T10:05:00Z", "asked", 0, "cannot_ask")];
        let eps = episodes_of(&rows);
        assert_eq!(
            eps.iter().map(|e| e.ended).collect::<Vec<_>>(),
            vec![EpisodeEnd::Withdrawn]
        );
    }

    #[test]
    fn a_reverted_rung_inside_an_episode_does_not_end_it() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:15:00Z", "continued", 1, "no_process"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(
            eps.iter().map(|e| e.ended).collect::<Vec<_>>(),
            vec![EpisodeEnd::Open]
        );
        assert_eq!(eps[0].resumes, vec![], "a resume taken back is no resume");
    }

    #[test]
    fn a_recommission_is_a_resume_and_its_outcome_is_the_next_row() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:15:00Z", "stopped", 0, "stop_requested"),
            row(
                3,
                "2026-10-24T10:17:00Z",
                "recommissioned",
                1,
                "session_gone_recommissioned",
            ),
            row(4, "2026-10-24T10:21:00Z", "recovered", 1, "life"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].resumes, vec![(1, ResumeOutcome::Recovered)]);
        assert_eq!(
            (eps[0].ended, eps[0].silence_to_recovery_secs),
            (EpisodeEnd::Recovered, Some(1260))
        );
    }

    #[test]
    fn a_reverted_recommission_is_no_resume_and_alone_is_an_episode_withdrawn() {
        let rows = [
            row(1, "2026-10-24T10:17:00Z", "recommissioned", 1, "no_process"),
            row(2, "2026-10-24T10:30:00Z", "asked", 1, "asked"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(
            eps.iter()
                .map(|e| (e.ended, e.resumes.len()))
                .collect::<Vec<_>>(),
            vec![(EpisodeEnd::Withdrawn, 0), (EpisodeEnd::Open, 0)]
        );
    }

    #[test]
    fn instants_with_an_offset_are_compared_as_instants() {
        // 10:30+02:00 is 08:30Z: twenty minutes after a silence that began at 08:10Z.
        let mut first = row(1, "2026-10-24T10:20:00+02:00", "asked", 0, "asked");
        first.silent_since = Some("2026-10-24T08:10:00Z".into());
        let rows = [
            first,
            row(2, "2026-10-24T10:30:00+02:00", "recovered", 0, "life"),
        ];
        assert_eq!(episodes_of(&rows)[0].silence_to_recovery_secs, Some(1200));
    }

    #[test]
    fn a_withdrawn_end_row_ends_the_episode() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:12:00Z", "withdrawn", 0, "withdrawn"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!((eps.len(), eps[0].ended), (1, EpisodeEnd::Withdrawn));
    }

    #[test]
    fn a_resume_followed_by_a_stop_was_dead_again_and_a_notice_between_is_passed_over() {
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            row(2, "2026-10-24T10:15:00Z", "stopped", 0, "stop_requested"),
            row(3, "2026-10-24T10:17:00Z", "continued", 1, "triggered"),
            row(4, "2026-10-24T10:20:00Z", "noticed", 1, "posted"),
            row(5, "2026-10-24T10:32:00Z", "stopped", 1, "stop_requested"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(eps[0].resumes, vec![(1, ResumeOutcome::DeadAgain)]);
    }

    #[test]
    fn slots_are_walked_apart() {
        let mut other = row(2, "2026-10-24T10:11:00Z", "asked", 0, "asked");
        other.role = "ws/verifier".into();
        let rows = [
            row(1, "2026-10-24T10:10:00Z", "asked", 0, "asked"),
            other,
            row(3, "2026-10-24T10:20:00Z", "recovered", 0, "life"),
        ];
        let eps = episodes_of(&rows);
        assert_eq!(
            eps.iter()
                .map(|e| (e.role.as_str(), e.ended))
                .collect::<Vec<_>>(),
            vec![
                ("ws/coder", EpisodeEnd::Recovered),
                ("ws/verifier", EpisodeEnd::Open)
            ]
        );
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two_rounded_down() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5, 1, 3]), Some(3));
        assert_eq!(median(&[4, 1, 2, 7]), Some(3));
    }
}
