// The tickets module (RFC 0002): v1's flow, in the repository space. Four collections, all
// written into the repository space only; the storage crate refuses them elsewhere.
//
// Every index runs in one space (`scope: "space"`), so a query reads one repository space even
// where a member holds several (RFC 0002, "Indexes").
//
// Left for a later ticket, an additive change: the ticket's `request` and its index `byRequest`
// (RFC 0009), once `requestsRequest` exists.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

/** A link to the ticket an object belongs to. */
const ticket = () => nc.link("ticketsTicket");

export const tickets = {
  // One object per ticket, epics included (RFC 0002, "The ticket"). The edges inside the space
  // are its fields; the references out of it are `ticketsReference` objects.
  ticketsTicket: nc.collection(
    {
      // The readable id, the creating member's tag, a dot and a number, such as `ka.42`; set
      // once, when the ticket is created (RFC 0002, "Ticket ids").
      key: nc.string(),
      type: nc.oneOf(["epic", "bug", "feature", "chore", "decision", "question"]),
      title: nc.string(),
      // Why the ticket exists and its goal.
      description: nc.string(),
      // The path to the goal; agent notes.
      design: nc.string(),
      // The acceptance criteria; v1's `completion_criterion`.
      completionCriterion: nc.string(),
      status: nc.oneOf(["open", "in_progress", "closed"]),
      // Why a closed ticket is closed; unset while it is open.
      resolution: nc.oneOf(["done", "canceled", "replaced"]),
      // For `replaced`: the ticket that carries the work on.
      replacedBy: ticket(),
      // The reason given at close.
      closingComment: nc.string(),
      // RFC 3339 instant of the last close.
      closedAt: nc.string(),
      // RFC 3339 instant of archiving; unset when not archived.
      archivedAt: nc.string(),
      // 0 to 4, v1's P0 to P4; unset means not set.
      priority: nc.int(),
      // RFC 3339.
      due: nc.string(),
      // RFC 3339.
      deferUntil: nc.string(),
      // Who holds the ticket: `member:<member id>`, `account:<account fingerprint>`, or v1's
      // free text as the move wrote it (RFC 0002, "Who holds a ticket").
      assignee: nc.string(),
      // The epic the ticket belongs to.
      parent: ticket(),
      // The tickets that block it.
      dependsOn: nc.links("ticketsTicket"),
      // Tickets it contributes to, without blocking.
      contributesTo: nc.links("ticketsTicket"),
      // Tickets it cites or is related to.
      mentions: nc.links("ticketsTicket"),
      // The team's labels, such as `rfc`.
      labels: nc.set(nc.string()),
      // RFC 3339 instants of the create and of the last write, written by `nxs`.
      createdAt: nc.string(),
      updatedAt: nc.string(),
      // The member id of the member that created the ticket, never changed; `v1:` and the v1
      // author for a moved ticket (RFC 0002, "Who created a ticket").
      createdBy: nc.string(),
      // The member id of the last writer: RFC 0001's author stop-gap.
      updatedBy: nc.string(),
      // The v1 item id of a moved ticket, such as `6j6v.pzkb`, written when the move creates it
      // and never changed; unset for a ticket created in v2 (RFC 0008, "How the ops are
      // replayed").
      v1Id: nc.string(),
    },
    {
      indexes: {
        byKey: nc.index(["key"], { equal: 1, ...space }),
        byStatus: nc.index(["status", "archivedAt", "key"], { equal: 2, ...space }),
        byClosedAt: nc.index(
          ["status", "archivedAt", { field: "closedAt", direction: "desc" }],
          { equal: 2, ...space },
        ),
        byClosedAtAll: nc.index(["status", { field: "closedAt", direction: "desc" }], {
          equal: 1,
          ...space,
        }),
        byArchivedAt: nc.index([{ field: "archivedAt", direction: "desc" }], space),
        byParent: nc.index(["parent", "key"], { equal: 1, ...space }),
        byAssignee: nc.index(["assignee", "status"], { equal: 2, ...space }),
        byType: nc.index(["type", "status", "archivedAt"], { equal: 3, ...space }),
        byUpdatedAt: nc.index([{ field: "updatedAt", direction: "desc" }], space),
        byV1Id: nc.index(["v1Id"], { equal: 1, ...space }),
      },
    },
  ),

  // A note on a ticket: append-only text (RFC 0002, "Notes").
  ticketsNote: nc.collection(
    {
      ticket: ticket(),
      // The text, Markdown.
      body: nc.string(),
      // The note it answers, when it answers one.
      replyTo: nc.link("ticketsNote"),
      createdAt: nc.string(),
      // RFC 0001's author stop-gap.
      createdBy: nc.string(),
    },
    {
      indexes: {
        byTicket: nc.index(["ticket", "createdAt"], { equal: 1, ...space }),
      },
    },
  ),

  // A PR, branch or artifact linked to a ticket (RFC 0002, "Work links").
  ticketsWork: nc.collection(
    {
      ticket: ticket(),
      kind: nc.oneOf(["pr", "branch", "artifact"]),
      // The PR's or the artifact's URL.
      url: nc.string(),
      // For a PR or a branch: the branch name.
      branch: nc.string(),
      // For a PR: its state as last recorded.
      state: nc.oneOf(["open", "merged", "closed"]),
      // For a merged PR: the commit on the base branch, when known.
      commit: nc.string(),
      createdAt: nc.string(),
      // RFC 0001's author stop-gap.
      createdBy: nc.string(),
    },
    {
      indexes: {
        byTicket: nc.index(["ticket"], { equal: 1, ...space }),
        byBranch: nc.index(["branch"], { equal: 1, ...space }),
        byUrl: nc.index(["url"], { equal: 1, ...space }),
      },
    },
  ),

  // A reference from a ticket to an object in another space (RFC 0002, "References out of the
  // space").
  ticketsReference: nc.collection(
    {
      // The referring ticket.
      ticket: ticket(),
      // `serves` a goal, `cites` another repository's ticket, or `proposed`: a goal a model
      // proposed the ticket serves, which the owner has not yet confirmed and which never counts
      // for focus (RFC 0005, "What the re-evaluation stores").
      kind: nc.oneOf(["serves", "cites", "proposed"]),
      // The target's space id and object id: a ticket of another repository, or a goal in the
      // coordination space (RFC 0005, "A ticket serves a goal").
      target: nc.ref(["ticketsTicket", "goalsGoal"]),
      // The target's readable name when the reference was written; for a goal, its key and
      // title.
      label: nc.string(),
      createdAt: nc.string(),
      // RFC 0001's author stop-gap.
      createdBy: nc.string(),
    },
    {
      indexes: {
        byTicket: nc.index(["ticket", "kind"], { equal: 1, ...space }),
        byTarget: nc.index(["target", "kind"], { equal: 1, ...space }),
      },
    },
  ),
};
