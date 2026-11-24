// The decisions module (RFC 0003): v1's memory, in the repository space. Four collections, all
// written into the repository space only; the storage crate refuses them elsewhere. One decision
// is one `decisionsDecision` object and the topic, subject and supersession objects its single
// write creates with it, none of which is ever written again (RFC 0003, "Append-only").
//
// The three join collections exist because a set or many links cannot be indexed, and the
// filters `--topic`, `--ticket` and "superseded by" each need an index (RFC 0003, "The
// collections"). Every index runs in one space (`scope: "space"`).
//
// Left for later tickets, each an additive change: `goalsGoal` (RFC 0005) and `requestsRequest`
// (RFC 0009) as targets of `decisionsSubject.elsewhere`, once those collections exist.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

/** A link to the decision an object belongs to. */
const decision = () => nc.link("decisionsDecision");

export const decisions = {
  // One object per decision (RFC 0003, "decisionsDecision").
  decisionsDecision: nc.collection(
    {
      // `YYYY-MM-DD-<ticket>-<slug>`, the file name in `factory/decisions/` without `.md`;
      // written once (RFC 0003, "Handles").
      handle: nc.string(),
      // The day the decision was made, `YYYY-MM-DD`.
      date: nc.string(),
      // The heading's ticket part as written: `no ticket`, `NEX-9, NEX-10` or `NEX-6 to NEX-13`.
      tickets: nc.string(),
      // The decision in one line of at most 200 characters.
      title: nc.string(),
      // Who decided, as written: a name, a role such as `coder on NEX-36`, or a sentence.
      decidedBy: nc.string(),
      why: nc.string(),
      // The options not taken.
      insteadOf: nc.string(),
      // Everything below the fields, as Markdown; empty when there is none.
      body: nc.string(),
      // The URL of the pull request whose code the decision describes, if any.
      pr: nc.string(),
      // The member id of the writing member: RFC 0001's author stop-gap.
      author: nc.string(),
    },
    {
      indexes: {
        byDate: nc.index(
          [
            { field: "date", direction: "desc" },
            { field: "handle", direction: "desc" },
          ],
          space,
        ),
        byHandle: nc.index(["handle"], { equal: 1, ...space }),
      },
    },
  ),

  // One object per topic of a decision (RFC 0003, "decisionsTopic").
  decisionsTopic: nc.collection(
    {
      decision: decision(),
      // A lowercase tag of letters, digits and `-`.
      topic: nc.string(),
      // The decision's date, copied, so the index can order by it.
      date: nc.string(),
    },
    {
      indexes: {
        byTopic: nc.index(["topic", { field: "date", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
      },
    },
  ),

  // One object per ticket, goal or request a decision is about (RFC 0003, "decisionsSubject").
  decisionsSubject: nc.collection(
    {
      decision: decision(),
      // A ticket in the same space, when it exists there.
      ticket: nc.link("ticketsTicket"),
      // A subject in another space.
      elsewhere: nc.ref(["ticketsTicket"]),
      // The subject's human-readable id as written, such as `NEX-22`; always written.
      label: nc.string(),
      // The decision's date, copied.
      date: nc.string(),
    },
    {
      indexes: {
        byLabel: nc.index(["label", { field: "date", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
        byTicket: nc.index(["ticket"], { equal: 1, ...space }),
      },
    },
  ),

  // One object per decision that a newer one supersedes (RFC 0003, "decisionsSupersession").
  decisionsSupersession: nc.collection(
    {
      // The newer decision.
      decision: decision(),
      // The older decision it replaces.
      supersedes: decision(),
      // The newer decision's date, copied.
      date: nc.string(),
    },
    {
      indexes: {
        bySuperseded: nc.index(["supersedes"], { equal: 1, ...space }),
        byDate: nc.index([{ field: "date", direction: "desc" }], space),
      },
    },
  ),
};
