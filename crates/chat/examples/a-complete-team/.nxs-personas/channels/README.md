# Channels

The notes that headed `channels.yaml`, moved here by `nxs personas migrate` — each channel is a file of its own now.

The channels this project runs on. A work item walks planning -> coding -> merging; review is a
step inside the coding leg.

TIMEOUTS below are held by the nexus-flow background service — the only clock in the system. They
fire only while the service runs AND this workspace is registered with it (`nxs sync daemon
status`, `nxs sync bind`). Register the workspace once, or the timeouts below never fire.

WHAT A TIMEOUT BUYS: "settled" means complete OR stale. Without one, a member that goes silent
holds its round open forever — on `review`, with `expects: all`, one dead reviewer parks the
round and the coder waiting on it.
