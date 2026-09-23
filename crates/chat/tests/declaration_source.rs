//! Where a workspace's declarations live, and what every surface says when there are none
//! (nxf 6j6v.dvyq step 4 — THE break of that item, plus the two bootstrap decisions of 2026-08-14).
//!
//! Three things are under test here and they are deliberately in one file, because they are one
//! answer wearing three faces:
//!
//! 1. **The move.** Declarations belong in `<workspace-root>/.nxs-personas/`. A legacy `roles/`
//!    folder is still READ and REPORTED — and never written, never moved, never rewritten.
//! 2. **The resolution as a first-class result.** [`DeclarationSource`] travels with the catalogue,
//!    so "where did this come from" and "was a legacy folder found" are one structure and not two
//!    channels (the note of 2026-08-14, answering app-foundations).
//! 3. **The bootstrap.** `nxc init` leaves the folder behind, and the empty catalogue stops being
//!    anonymous: `list`/`prime` SAY it and exit 0, `send --to` REFUSES by name, and
//!    `Engine::definitions` returns an empty catalogue and succeeds.

use assert_cmd::Command;
use nexus_chat::definitions::{
    DeclarationSource, DeclarationSourceKind, Definitions, PERSONAS_README,
};
use nexus_chat::workspace::{ChatWorkspaceExt, LEGACY_ROLES_DIR, PERSONAS_DIR};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-16T10:00:00Z";

const PM: &str = "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n";
const CODER: &str = "handle: coder\njob_title: Implementer\nsystem_prompt: You are the coder.\n";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

/// Write one persona declaration into `dir`, creating it.
fn declare(dir: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), body).unwrap();
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env_remove("NXC_WORKER")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).into_owned()
}

fn json_of(cmd: &mut Command) -> Value {
    serde_json::from_str(stdout_of(cmd).trim()).expect("valid json")
}

// ---- the resolution itself -------------------------------------------------

#[test]
fn a_workspace_that_declares_nothing_resolves_to_the_personas_path_and_says_where_it_is() {
    let tmp = TempDir::new().unwrap();
    let defs = Definitions::resolve(tmp.path()).expect("an empty workspace resolves cleanly");
    let source = defs
        .source()
        .expect("a resolved catalogue names its source");

    assert!(defs.is_empty(), "nothing is declared");
    assert_eq!(source.kind, DeclarationSourceKind::None);
    assert_eq!(source.count, 0);
    // The path names where a declaration BELONGS, not what happens to be there — which is the whole
    // reason this resolution exists: an empty list is indistinguishable from "there is nothing".
    assert_eq!(source.path, tmp.path().join(PERSONAS_DIR));
    assert_eq!(source.legacy_path, None);
    assert!(
        source
            .explain()
            .contains(&source.path.display().to_string()),
        "the explanation carries the path: {}",
        source.explain()
    );
}

#[test]
fn declarations_are_read_from_the_personas_folder() {
    let tmp = TempDir::new().unwrap();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);

    let defs = Definitions::resolve(tmp.path()).unwrap();
    let source = defs.source().unwrap();
    assert_eq!(source.kind, DeclarationSourceKind::Personas);
    assert_eq!(source.count, 1);
    assert_eq!(source.read_dir, tmp.path().join(PERSONAS_DIR));
    assert_eq!(defs.role("pm").unwrap().handle, "pm");
}

/// The migration path (acceptance point 2): the engine READS a legacy folder and REPORTS it.
#[test]
fn a_legacy_roles_folder_is_read_and_reported_machine_readably() {
    let tmp = TempDir::new().unwrap();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);

    let defs = Definitions::resolve(tmp.path()).unwrap();
    let source = defs.source().unwrap();

    assert_eq!(defs.role("pm").unwrap().handle, "pm", "it is READ");
    assert_eq!(source.kind, DeclarationSourceKind::LegacyRoles);
    assert_eq!(source.read_dir, tmp.path().join(LEGACY_ROLES_DIR));
    assert_eq!(
        source.legacy_path,
        Some(tmp.path().join(LEGACY_ROLES_DIR)),
        "and REPORTED, by path"
    );
    // Machine-readable, not just a sentence: the same fact as data.
    let v = source.to_value();
    assert_eq!(v["kind"], "legacy_roles");
    assert_eq!(v["count"], 1);
    assert_eq!(
        v["legacy_path"],
        Value::String(tmp.path().join(LEGACY_ROLES_DIR).display().to_string())
    );
    // The path a declaration BELONGS at is still the new one — the break is untouched by the
    // reading compatibility.
    assert_eq!(
        v["path"],
        Value::String(tmp.path().join(PERSONAS_DIR).display().to_string())
    );
}

/// Reading is reading: the folder that was found is not touched, and no new one appears in
/// somebody else's project just because an app opened it.
#[test]
fn resolving_a_legacy_workspace_writes_nothing_at_all() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(LEGACY_ROLES_DIR);
    declare(&legacy, "pm.yaml", PM);
    let before = std::fs::read_to_string(legacy.join("pm.yaml")).unwrap();

    Definitions::resolve(tmp.path()).unwrap();

    assert_eq!(
        std::fs::read_to_string(legacy.join("pm.yaml")).unwrap(),
        before,
        "the legacy declaration is untouched"
    );
    assert!(
        !tmp.path().join(PERSONAS_DIR).exists(),
        "and no `{PERSONAS_DIR}` is conjured into a project that has merely been opened"
    );
    assert_eq!(
        std::fs::read_dir(tmp.path()).unwrap().count(),
        1,
        "nothing else appeared beside it either"
    );
}

#[test]
fn the_personas_folder_wins_when_both_carry_declarations() {
    let tmp = TempDir::new().unwrap();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "coder.yaml", CODER);

    let defs = Definitions::resolve(tmp.path()).unwrap();
    let source = defs.source().unwrap();

    assert_eq!(source.kind, DeclarationSourceKind::Personas);
    assert_eq!(
        source.count, 1,
        "one catalogue, from one folder — not a merge"
    );
    assert!(
        defs.role("pm").is_ok(),
        "the personas folder is the one read"
    );
    assert!(
        defs.role("coder").is_err(),
        "the legacy folder loses the tie outright; a merge would have both"
    );
    assert_eq!(
        source.legacy_path,
        Some(tmp.path().join(LEGACY_ROLES_DIR)),
        "the loser is still REPORTED — an app offering the move has to know it is there"
    );
}

/// The reason "wins the tie" is decided on CONTENT and not on existence: `nxc init` now creates an
/// empty `.nxs-personas/`, and an existence-only rule would let running `init` in a legacy
/// workspace silently unteam it.
#[test]
fn an_empty_personas_folder_does_not_shadow_a_legacy_one() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join(PERSONAS_DIR)).unwrap();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);

    let defs = Definitions::resolve(tmp.path()).unwrap();
    assert_eq!(
        defs.source().unwrap().kind,
        DeclarationSourceKind::LegacyRoles
    );
    assert_eq!(defs.role("pm").unwrap().handle, "pm");
}

/// The `README.md` `nxc init` drops in is prose, not a declaration — it must not make an empty
/// folder look occupied and shadow a legacy one.
#[test]
fn the_init_readme_does_not_count_as_a_declaration() {
    let tmp = TempDir::new().unwrap();
    let personas = tmp.path().join(PERSONAS_DIR);
    std::fs::create_dir_all(&personas).unwrap();
    std::fs::write(personas.join(PERSONAS_README), "# .nxs-personas\n").unwrap();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);

    assert_eq!(
        Definitions::resolve(tmp.path())
            .unwrap()
            .source()
            .unwrap()
            .kind,
        DeclarationSourceKind::LegacyRoles
    );
}

// `a_workflows_subdirectory_alone_makes_a_folder_carry_declarations` stood here: a `workflows/`
// subdirectory alone was enough to make a folder count as declaring something. REMOVED with the
// run engine (6j6v.dvyq §3) — `carries_declarations` no longer looks into it, so a folder holding
// nothing but those files declares no team, which is now the truth rather than a regression.

/// A malformed file in the folder that LOSES must not fail a workspace whose declarations are fine
/// — which is why the resolution is parse-free.
#[test]
fn a_broken_legacy_folder_cannot_break_a_workspace_that_has_moved_on() {
    let tmp = TempDir::new().unwrap();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    declare(
        &tmp.path().join(LEGACY_ROLES_DIR),
        "broken.yaml",
        ": : not yaml at all : :\n",
    );

    let defs = Definitions::resolve(tmp.path()).expect("the losing folder is never parsed");
    assert_eq!(defs.source().unwrap().kind, DeclarationSourceKind::Personas);
}

/// A catalogue built in memory has no folder to have come from, and says so rather than inventing
/// a path — the honest shape of `None` (see the field's own doc).
#[test]
fn an_in_memory_catalogue_carries_no_source() {
    let defs = Definitions::new(Vec::new(), Vec::new()).unwrap();
    assert!(defs.source().is_none());
}

// ---- the four surfaces, each in its own register ---------------------------

/// `Engine::definitions()` — a READ of a catalogue. Its correct answer to "there are no
/// declarations" is "there are no declarations". app-foundations has this pinned on their side
/// (`engine-bridge/src/chat.rs`, `engine-server/tests/chat_wire.rs`); this is the same claim,
/// upstream of it, plus the resolution the empty answer now carries.
#[test]
fn the_engine_reads_an_empty_catalogue_successfully_and_names_its_source() {
    let tmp = workspace();
    let engine = nexus_chat::engine::Engine::open(None, tmp.path()).unwrap();
    let defs = engine
        .definitions()
        .expect("a read that fails BECAUSE there is nothing to read is a bad read");
    assert!(defs.is_empty());
    let source = defs
        .source()
        .expect("Ok, empty, WITH the source resolution — the 2026-08-14 table's first row");
    assert_eq!(source.kind, DeclarationSourceKind::None);
    assert_eq!(source.path, tmp.path().join(PERSONAS_DIR));
}

#[test]
fn the_engine_reads_a_legacy_workspace_and_reports_it() {
    let tmp = workspace();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);
    let engine = nexus_chat::engine::Engine::open(None, tmp.path()).unwrap();
    let defs = engine.definitions().unwrap();
    assert_eq!(defs.role("pm").unwrap().handle, "pm");
    assert_eq!(
        defs.source().unwrap().kind,
        DeclarationSourceKind::LegacyRoles
    );
}

/// `list` is ORIENTATION: it does not fail, it says so — with the path.
#[test]
fn list_says_where_a_declaration_belongs_and_exits_zero() {
    let tmp = workspace();
    let out = stdout_of(nxc(&tmp).args(["list"]));
    assert!(out.contains("nobody is declared here yet"), "{out}");
    assert!(
        out.contains(PERSONAS_DIR),
        "the path a declaration belongs at, not just the absence: {out}"
    );
}

#[test]
fn list_json_carries_the_resolution_beside_the_empty_directory() {
    let tmp = workspace();
    let v = json_of(nxc(&tmp).args(["--json", "list"]));
    assert_eq!(v["declarations"]["kind"], "none");
    assert_eq!(v["declarations"]["count"], 0);
    // Compared by SUFFIX, not verbatim: the subprocess resolves the temp dir through macOS's
    // `/private` symlink, so the two spellings of the same directory are both correct.
    assert!(
        v["declarations"]["path"]
            .as_str()
            .unwrap()
            .ends_with(PERSONAS_DIR),
        "the app renders its empty list WITH the path beside it, instead of rebuilding it: {v}"
    );
}

#[test]
fn list_tells_a_human_when_the_team_it_just_printed_came_from_the_legacy_folder() {
    let tmp = workspace();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);
    let out = stdout_of(nxc(&tmp).args(["list"]));
    assert!(out.contains("pm"), "the team is still printed: {out}");
    assert!(
        out.contains("LEGACY") && out.contains(PERSONAS_DIR),
        "and the move it is waiting for is named: {out}"
    );
}

/// `prime` is orientation too — same register, same exit code.
#[test]
fn prime_says_so_when_nothing_is_declared_and_exits_zero() {
    let tmp = workspace();
    let out = stdout_of(nxc(&tmp).args(["prime"]));
    assert!(out.contains("## Declarations"), "{out}");
    assert!(out.contains(PERSONAS_DIR), "{out}");
}

#[test]
fn prime_json_always_carries_the_declaration_source() {
    let tmp = workspace();
    let v = json_of(nxc(&tmp).args(["--json", "prime"]));
    assert_eq!(v["declarations"]["kind"], "none");

    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    let v = json_of(nxc(&tmp).args(["--json", "prime"]));
    assert_eq!(
        v["declarations"]["kind"], "personas",
        "present when the workspace is furnished too — the question is the same either way"
    );
    assert_eq!(v["declarations"]["count"], 1);
}

/// A furnished `.nxs-personas/` with no legacy folder is the ORDINARY case, and prime stays silent
/// about it — a paragraph repeating the path in every session-start block forever is noise.
#[test]
fn prime_markdown_stays_silent_about_an_ordinary_personas_folder() {
    let tmp = workspace();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    let out = stdout_of(nxc(&tmp).args(["prime"]));
    assert!(out.contains("pm"), "the team is there: {out}");
    assert!(!out.contains("## Declarations"), "{out}");
}

/// `send --to` is the ONE surface that genuinely fails: an action with a mandatory target has none.
#[test]
fn send_to_refuses_by_name_when_nothing_is_declared() {
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["send", "--to", "pm", "go"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("nothing is declared in this workspace"),
        "a NAMED rejection, not `no such target: pm` — there was never a target to typo: {stderr}"
    );
    assert!(
        stderr.contains(PERSONAS_DIR),
        "and it says where one goes: {stderr}"
    );
}

/// The named rejection is for the EMPTY workspace only. A furnished one that simply does not
/// declare `ghost` still gets the "no such target" answer, which is the useful one there.
#[test]
fn send_to_still_says_no_such_target_when_the_workspace_is_furnished() {
    let tmp = workspace();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    let out = nxc(&tmp)
        .args(["send", "--to", "ghost", "go"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(stderr.contains("no such target: ghost"), "{stderr}");
    assert!(
        !stderr.contains("nothing is declared"),
        "the bootstrap message must not swallow the ordinary miss: {stderr}"
    );
}

/// A channel that EXISTS but that no declaration names is refused BY NAME too — the third face of
/// the same answer (nxf 6j6v.dvyq §3, the raw channels). Before the raw channels went, this was a
/// legitimate target; now it is not, and "no such target" would send the caller hunting for a typo
/// in an id that is spelled perfectly and really is there.
///
/// The direct conversation `send --to <persona>` opens is the only such channel a workspace can
/// still produce, which is exactly why it is the fixture: there is no `channels create` any more.
#[test]
fn send_to_names_a_channel_that_exists_but_no_declaration_names() {
    let tmp = workspace();
    declare(&tmp.path().join(PERSONAS_DIR), "pm.yaml", PM);
    let opened = json_of(nxc(&tmp).args(["--json", "send", "--to", "pm", "go"]));
    let dm = opened["channel"]
        .as_str()
        .expect("the receipt names the direct conversation it opened")
        .to_string();

    let out = nxc(&tmp)
        .args(["send", "--to", &dm, "again"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(&dm) && stderr.contains("no declaration names it"),
        "the id is named and so is the reason: {stderr}"
    );
    assert!(
        stderr.contains(PERSONAS_DIR),
        "and it says where a declaration goes: {stderr}"
    );
    assert!(
        !stderr.contains("no such target"),
        "this channel is not a typo — it exists: {stderr}"
    );
}

// ---- bootstrap: `nxc init` leaves the folder behind ------------------------

#[test]
fn init_creates_the_personas_folder_with_an_explanation_and_no_shipped_personas() {
    let tmp = TempDir::new().unwrap();
    nxc(&tmp).args(["init", "--quiet"]).assert().success();

    let personas = tmp.path().join(PERSONAS_DIR);
    assert!(
        personas.is_dir(),
        "init leaves the place a declaration goes"
    );
    let readme = std::fs::read_to_string(personas.join(PERSONAS_README))
        .expect("with a short explanation of the format inside");
    assert!(readme.contains("<handle>.yaml"), "{readme}");
    assert!(readme.contains("channels.yaml"), "{readme}");

    // NO shipped personas: those are explicitly outside this epic (owner note on 6j6v.m4xe), and a
    // workspace that arrived with a team would make the whole declaration story a lie.
    let yaml: Vec<_> = std::fs::read_dir(&personas)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "yaml"))
        .collect();
    assert!(yaml.is_empty(), "no declarations are shipped: {yaml:?}");
    assert!(
        Definitions::resolve(tmp.path()).unwrap().is_empty(),
        "so the freshly-initialized workspace still resolves to an empty catalogue"
    );
}

#[test]
fn init_reports_the_personas_folder_it_created() {
    let tmp = TempDir::new().unwrap();
    let v = json_of(nxc(&tmp).args(["--json", "init"]));
    // By suffix — see `list_json_carries_the_resolution_beside_the_empty_directory`.
    assert!(
        v["personas"].as_str().unwrap().ends_with(PERSONAS_DIR),
        "{v}"
    );
}

#[test]
fn a_second_init_keeps_an_edited_explainer_and_every_declaration() {
    let tmp = TempDir::new().unwrap();
    nxc(&tmp).args(["init", "--quiet"]).assert().success();
    let personas = tmp.path().join(PERSONAS_DIR);
    std::fs::write(personas.join(PERSONAS_README), "my own notes\n").unwrap();
    std::fs::write(personas.join("pm.yaml"), PM).unwrap();

    nxc(&tmp).args(["init", "--quiet"]).assert().success();

    assert_eq!(
        std::fs::read_to_string(personas.join(PERSONAS_README)).unwrap(),
        "my own notes\n",
        "an edited explainer survives"
    );
    assert!(personas.join("pm.yaml").exists(), "and so does the team");
}

/// `init` sets up THIS workspace; it does not migrate anybody's project for them.
#[test]
fn init_never_moves_or_touches_a_legacy_folder() {
    let tmp = TempDir::new().unwrap();
    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);

    nxc(&tmp).args(["init", "--quiet"]).assert().success();

    assert!(
        tmp.path().join(LEGACY_ROLES_DIR).join("pm.yaml").exists(),
        "the legacy folder is left exactly where it was"
    );
    // And the empty folder `init` just made does not shadow it: the workspace still has its team.
    let defs = Definitions::resolve(tmp.path()).unwrap();
    assert_eq!(defs.role("pm").unwrap().handle, "pm");
    assert_eq!(
        defs.source().unwrap().kind,
        DeclarationSourceKind::LegacyRoles
    );
}

// ---- the workspace seam ----------------------------------------------------

#[test]
fn the_workspace_resolves_both_locations_and_the_one_that_is_read() {
    let tmp = workspace();
    let ws = nexus_chat::workspace::Workspace::resolve(None, tmp.path()).unwrap();
    assert_eq!(ws.personas_dir().unwrap(), tmp.path().join(PERSONAS_DIR));
    assert_eq!(
        ws.legacy_roles_dir().unwrap(),
        tmp.path().join(LEGACY_ROLES_DIR)
    );
    // Nothing declared: the folder to read is the one declarations belong in, which is what keeps
    // the resolution total (every loader reads a missing folder as an empty catalogue).
    assert_eq!(ws.declaration_dir().unwrap(), tmp.path().join(PERSONAS_DIR));

    declare(&tmp.path().join(LEGACY_ROLES_DIR), "pm.yaml", PM);
    assert_eq!(
        ws.declaration_dir().unwrap(),
        tmp.path().join(LEGACY_ROLES_DIR),
        "and it follows the same rule the catalogue does"
    );
}

#[test]
fn the_resolution_is_total_over_a_root_that_does_not_exist() {
    // A path that does not exist at all is not an error — it is a workspace with nothing declared.
    // (A root that cannot be canonicalized keeps the caller's spelling; there is no other answer,
    // and it is never a path anything was read from.)
    let source = DeclarationSource::resolve(Path::new("/nonexistent/project")).unwrap();
    assert_eq!(source.kind, DeclarationSourceKind::None);
    assert_eq!(source.legacy_path, None);
    assert_eq!(source.count, 0);
    assert_eq!(
        source.read_dir,
        Path::new("/nonexistent/project").join(PERSONAS_DIR)
    );
}

/// The human spelling of a source kind and its wire spelling are one fact; a second spelling is how
/// two views drift apart.
#[test]
fn the_kind_reads_the_same_to_a_human_and_on_the_wire() {
    for kind in [
        DeclarationSourceKind::Personas,
        DeclarationSourceKind::LegacyRoles,
        DeclarationSourceKind::None,
    ] {
        assert_eq!(
            Value::String(kind.as_str().to_string()),
            serde_json::to_value(kind).unwrap(),
            "{kind:?}"
        );
    }
}
