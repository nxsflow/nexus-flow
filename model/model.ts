// nexus-flow's NoCentralDB model (RFC 0001, "The model"): one model for the whole app, with
// every collection of every module. `ncdb model build` writes model.lock.json from it, and the
// Rust code reads only that lock, so building nxs needs neither Node nor ncdb.
//
// A collection's name is its module's name in lower case followed by the collection's own name
// in upper camel case; the frame's own collections use the prefix `space`. The model only grows:
// a field or collection, once on v2, is never renamed or removed.
import { nc } from "@nocentraldb/sdk";

export const model = nc.model({
  // The role record (RFC 0001, "The role record"): one record per member nexus-flow added to a
  // space, written by the member that adds it.
  spaceMember: nc.collection({
    // The NoCentralDB member id.
    member: nc.string(),
    // `owner`, `developer`, `pm`, `app`, `migration`, or the role of a session (`coder`, ...).
    role: nc.string(),
    // For a session: the ticket it was started on, if any. It refers only to collections that
    // exist, so it has no target until the tickets module declares its collection; adding a
    // target is additive.
    ticket: nc.ref([]),
  }),
});
