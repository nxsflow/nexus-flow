// The status module (RFC 0007): the daily notes, in the repository space. Three collections, written
// into the repository space only; the storage crate refuses them elsewhere (RFC 0007, "Where a note
// lives"). One note is one `statusNote` and the `statusItem` and `statusGoal` objects its single
// write creates with it, and none of them takes a change after its create ("Correcting a note").
//
// Every index runs in one space (`scope: "space"`) but `statusNote.byDateAcrossSpaces`, which spans
// every space of the member, so that the owner reads every repository's notes of a day in one page.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

const health = ["onTrack", "atRisk", "offTrack"];

export const status = {
  // One object per note (RFC 0007, "statusNote").
  statusNote: nc.collection(
    {
      // The day the note reports on, `YYYY-MM-DD`, in `timeZone`.
      date: nc.string(),
      // The IANA time zone the day is counted in, such as `Europe/Berlin`.
      timeZone: nc.string(),
      // The start of the period the note covers, RFC 3339 in UTC.
      since: nc.string(),
      // The end of the period, RFC 3339 in UTC: when the note's facts were gathered.
      until: nc.string(),
      // The repository's short name, as RFC 0009 forms it.
      repository: nc.string(),
      // The note's health: the computed one, or the writer's when it overrides it; unset when
      // neither gives one (RFC 0007, "Health").
      health: nc.oneOf(health),
      // The health the rule computed when the note was written; unset when it had no goal to go by.
      healthComputed: nc.oneOf(health),
      // Why the writer's health differs from the computed one; set only when it does.
      healthWhy: nc.string(),
      // Why the rule computed no health; unset when it computed one.
      healthBasis: nc.oneOf(["noCoordinationSpace", "noGoalInFocus", "noWorkForGoals"]),
      // The note in one line of at most 200 characters.
      headline: nc.string(),
      // The note's text, Markdown, as the team's template asks for it.
      body: nc.string(),
      // The earlier note this note corrects, if any (RFC 0007, "Correcting a note").
      corrects: nc.link("statusNote"),
      // The member id of the member that wrote the note: RFC 0001's author stop-gap.
      author: nc.string(),
    },
    {
      indexes: {
        byDate: nc.index(
          [
            { field: "date", direction: "desc" },
            { field: "until", direction: "desc" },
          ],
          space,
        ),
        // The one index without `scope: "space"`: every repository's notes of a day at once.
        byDateAcrossSpaces: nc.index([
          { field: "date", direction: "desc" },
          { field: "until", direction: "desc" },
        ]),
        byCorrects: nc.index(["corrects"], { equal: 1, ...space }),
      },
    },
  ),
  // One object per line of a note that reports on something (RFC 0007, "statusItem"). A line
  // points at one record or at none.
  statusItem: nc.collection(
    {
      // The note.
      note: nc.link("statusNote"),
      // Which part of the note it belongs to.
      kind: nc.oneOf(["changed", "works", "waits", "decided", "blocked"]),
      // Its place within its kind, from 1.
      position: nc.int(),
      // The line, one line of at most 200 characters.
      text: nc.string(),
      // A ticket of this repository, when the line is about one.
      ticket: nc.link("ticketsTicket"),
      // A decision of this repository, when the line is about one.
      decision: nc.link("decisionsDecision"),
      // A pull request's URL, when the line is about one.
      pr: nc.string(),
      // A goal in the coordination space, or another repository's ticket, when the line is about
      // one. RFC 0007 also names `requestsRequest`, which RFC 0009 adds when it declares it.
      elsewhere: nc.ref(["goalsGoal", "ticketsTicket"]),
      // The target's readable name when the line was written.
      label: nc.string(),
      // The note's date, copied, so the indexes can order by it.
      date: nc.string(),
    },
    {
      indexes: {
        byNote: nc.index(["note", "kind", "position"], { equal: 1, ...space }),
        byTicket: nc.index(["ticket", { field: "date", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
        byDecision: nc.index(["decision", { field: "date", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
      },
    },
  ),
  // One object per goal in focus the note's health was computed from (RFC 0007, "Health"): what
  // the rule saw, kept with the note, since the tickets it counted move on.
  statusGoal: nc.collection(
    {
      // The note.
      note: nc.link("statusNote"),
      // The goal in the coordination space.
      goal: nc.ref(["goalsGoal"]),
      // The goal's key and title when the note was written.
      label: nc.string(),
      // Its place in the order then, `1` for the first.
      position: nc.int(),
      // Its verdict.
      verdict: nc.oneOf(["onTrack", "atRisk", "offTrack", "noWork"]),
      // The counts the verdict was computed from.
      open: nc.int(),
      inProgress: nc.int(),
      ready: nc.int(),
      blocked: nc.int(),
      doneInPeriod: nc.int(),
      overdue: nc.int(),
      flagged: nc.int(),
      waitingForOwner: nc.int(),
      // The note's date, copied.
      date: nc.string(),
    },
    {
      indexes: {
        byNote: nc.index(["note", "position"], { equal: 1, ...space }),
        byGoal: nc.index(["goal", { field: "date", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
      },
    },
  ),
};
