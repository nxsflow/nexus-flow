// The move from v1 (RFC 0008): one object per run of `nxs migrate`, and one record per v1 op or
// decision file the run moved, in the repository space only; the storage crate refuses them
// elsewhere. Only the move writes either collection, through the store's write path for the
// move. A record is the record of what happened: the storage crate refuses every write to an
// existing one (RFC 0008, "The migration records"). A run changes as it goes on.
//
// Every index runs in one space (`scope: "space"`).
//
// Left for a later ticket: declaring `migrationRecord` write-once in the model (R12), once
// NoCentralDB offers it.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

export const migration = {
  // One object per run (RFC 0008, "The migration run").
  migrationRun: nc.collection(
    {
      // The v1 workspace's identity: its `sync.toml` stream id when it is bound to one, else its
      // `replica_uuid`.
      workspace: nc.string(),
      // The `replica_uuid` and `prefix` of the replica the run read.
      replica: nc.string(),
      // The migration member's id.
      member: nc.string(),
      // RFC 3339.
      startedAt: nc.string(),
      endedAt: nc.string(),
      state: nc.oneOf(["replaying", "verifying", "complete", "verificationFailed", "undone"]),
      // How many ops the run will replay.
      ops: nc.int(),
      // The position of the last op written.
      last: nc.int(),
      // The run's report as JSON, once it ends.
      report: nc.string(),
    },
    {
      indexes: {
        byWorkspace: nc.index(["workspace", "startedAt"], { equal: 1, ...space }),
      },
    },
  ),

  // One object per v1 op or decision file a run moved (RFC 0008, "The migration records").
  migrationRecord: nc.collection(
    {
      // The run that wrote it.
      run: nc.link("migrationRun"),
      // A v1 op, or a decision file.
      source: nc.oneOf(["op", "file"]),
      // The v1 `op_id`, or the file's path.
      origin: nc.string(),
      // The op's place in v1's order within the run.
      position: nc.int(),
      // The op's domain, target kind, op type and field, joined by `/`, or `decision-file`.
      kind: nc.string(),
      // The v1 item id or memory key the op is about, or the decision file's name.
      item: nc.string(),
      // The ticket the op wrote to, for a flow op.
      ticket: nc.link("ticketsTicket"),
      // The op's `author`, as v1 wrote it.
      author: nc.string(),
      // The op's `wall_clock`, RFC 3339.
      wallClock: nc.string(),
      // The op's v1 coordinates.
      lamport: nc.int(),
      site: nc.int(),
      // The op's v1 signature envelope, unset for an unsigned op.
      keyId: nc.string(),
      sig: nc.string(),
      // The verdict of the replica the move read.
      provenance: nc.oneOf(["own", "verified", "invalid", "unsigned"]),
      // The op's canonical bytes, base64url; unset for a withheld op and for a file.
      payload: nc.string(),
      // What the move did with it.
      outcome: nc.oneOf(["applied", "noEffect", "dropped", "withheld", "lateConflict"]),
    },
    {
      indexes: {
        // Finds an op already moved.
        byOrigin: nc.index(["origin"], { equal: 1, ...space }),
        // A ticket's v1 history in order.
        byTicket: nc.index(["ticket", "position"], { equal: 1, ...space }),
        // The same for a memory or an item.
        byItem: nc.index(["item", "position"], { equal: 1, ...space }),
        byRun: nc.index(["run", "position"], { equal: 1, ...space }),
      },
    },
  ),
};
