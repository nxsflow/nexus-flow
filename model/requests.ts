// The requests module (RFC 0009): the work a group of repositories asks of each other, in its
// coordination space. Its collections are written into the coordination space only; the storage
// crate refuses them elsewhere (RFC 0009, "The coordination space").
//
// Every index runs in one space (`scope: "space"`), the coordination space a record is read from.
import { nc } from "@nocentraldb/sdk";

const space = { scope: "space" } as const;

export const requests = {
  // One object per repository of the group (RFC 0009, "Repositories in the space"), so that a
  // request can be addressed to it by name. A record is never deleted: a repository that leaves
  // the group sets `left`, and requests to and from it stay readable.
  requestsRepository: nc.collection(
    {
      // The repository's short name, such as `central-infra`; by default the last path segment
      // of the git remote `origin`, without `.git`.
      name: nc.string(),
      // The repository space's id, as `nexus-flow.toml` names it.
      space: nc.string(),
      // Set when the repository has left the group; unset otherwise.
      left: nc.boolean(),
    },
    {
      indexes: {
        // Every record in name order: `nxs repository list`, and the records that hold a name.
        byName: nc.index(["name", "space"], { equal: 0, ...space }),
        // A repository's own records, found by its space when it joins again or leaves.
        bySpace: nc.index(["space"], { equal: 1, ...space }),
      },
    },
  ),
};
