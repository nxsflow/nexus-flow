// nexus-flow's NoCentralDB model (RFC 0001, "The model"): one model for the whole app, with
// every collection of every module. `ncdb model build` writes model.lock.json from it, and the
// Rust code reads only that lock, so building nxs needs neither Node nor ncdb.
//
// A collection's name is its module's name in lower case followed by the collection's own name
// in upper camel case; the frame's own collections use the prefix `space`. The model only grows:
// a field or collection, once on v2, is never renamed or removed.
import { nc } from "@nocentraldb/sdk";

import { decisions } from "./decisions.ts";
import { goals } from "./goals.ts";
import { tickets } from "./tickets.ts";

export const model = nc.model({
  // The role record (RFC 0001, "The role record"): one record per member nexus-flow added to a
  // space, written by the member that adds it.
  spaceMember: nc.collection({
    // The NoCentralDB member id.
    member: nc.string(),
    // `owner`, `developer`, `pm`, `app`, `migration`, or the role of a session (`coder`, ...).
    role: nc.string(),
    // For a session: the ticket it was started on, if any.
    ticket: nc.ref(["ticketsTicket"]),
    // The member's ticket tag, two characters such as `ka`, which starts the keys of the
    // tickets it creates; unset for a member that has none (RFC 0002, "Ticket ids").
    tag: nc.string(),
  }, {
    indexes: {
      // A member's record in one space: `nxs member list` reads the role of each current member
      // by it, and `nxs member add` finds the record of a member it adds again.
      byMember: nc.index(["member"], { equal: 1, scope: "space" }),
    },
  }),
  ...tickets,
  ...decisions,
  ...goals,
});
