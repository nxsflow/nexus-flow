// The goals module (RFC 0005): the goals a group of repositories focuses on, in its coordination
// space. One collection, written into the coordination space only; the storage crate refuses it
// elsewhere (RFC 0005, "Where goals live").
//
// Every index runs in one space (`scope: "space"`), the coordination space a goal is read from.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

export const goals = {
  // One object per goal (RFC 0005, "The goal"). A goal is never deleted: one that no longer
  // matters is `dropped`. It holds no list of the tickets that serve it; each ticket names the
  // goal through a `ticketsReference` in its repository space.
  goalsGoal: nc.collection(
    {
      // The readable id, the creating member's tag in the coordination space, a dot and a
      // number, such as `cb.3`; set once, when the goal is created (RFC 0005, "Goal ids").
      key: nc.string(),
      // The goal in one line.
      title: nc.string(),
      // Why it matters, Markdown; optional.
      description: nc.string(),
      // What "reached" means, as a check a person can make; optional.
      reachedWhen: nc.string(),
      // Where the goal stands; `nxs` reads a goal with no status as `later` (RFC 0005, "Status").
      status: nc.oneOf(["proposed", "focus", "later", "reached", "dropped"]),
      // Its place among the goals in focus, compared by its bytes; unset for a goal not in focus
      // (RFC 0005, "The order of the goals in focus").
      rank: nc.string(),
      // RFC 3339 UTC instant since which it has had this status.
      statusAt: nc.string(),
      // Why the status last changed, if a reason was given.
      statusComment: nc.string(),
      // Who answers for the goal: `account:<fingerprint>` or `member:<member id>`, as RFC 0002's
      // `assignee`; optional.
      owner: nc.string(),
      // RFC 3339 instant of the create.
      createdAt: nc.string(),
      // The member id of the member that created the goal: RFC 0001's author stop-gap.
      createdBy: nc.string(),
      // The member id of the member that set the status and the rank.
      statusBy: nc.string(),
    },
    {
      indexes: {
        // The goals of one status: those in focus in their order, the others longest-standing
        // first, since only a goal in focus has a rank.
        byStatus: nc.index(["status", "rank", "statusAt"], { equal: 1, ...space }),
        byKey: nc.index(["key"], { equal: 1, ...space }),
      },
    },
  ),
};
