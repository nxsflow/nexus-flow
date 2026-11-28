# 2026-11-28 · NEX-247 · Minting before sync or without a tag has its own error codes

- Decided by: Carsten
- Why: the store refuses to mint a ticket key when the member holds a freed tag and its copy has not synced yet, or when the writing member's record has no tag, and RFC 0002 named no error code for either. Each case has a different remedy, and an agent acts on the code, not the message, so `nxs` prints `not_synced` (run `nxs sync`, then try again) and `no_tag` (run `nxs member retag`). RFC 0002, "Commands, JSON and the seam", says so.
- Instead of: (b) `validation` for both, with the cause only in the message.
- Topics: tickets, errors, v2
