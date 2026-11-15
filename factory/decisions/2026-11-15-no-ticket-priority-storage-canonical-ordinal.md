# 2026-11-15 · no ticket · Priority is a plugin-named label at the SURFACE but stored as the canonical ORDINAL — never store the label, or --json stops being plugin-independent.

- Decided by: recorded in the project memory (`nxm`, key `priority-storage-canonical-ordinal`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: flow, plugins, json-contract

## The record

Priority is a named plugin variant at the SURFACE but stored as the canonical ORDINAL.

Decision pivot: the label string was stored first, and that broke the gate asserting that the
canonical `--json` record is plugin-INDEPENDENT (`output.rs` never consults a plugin; only
presentation and ranking may differ). It is the faithful analogue of `vocabulary.status`, which
likewise stores a plugin-independent core token.

So: `--priority`/`--set` take a LABEL (`P0..P4` / `now..icebox`), resolved to its index in
`cfg.priority.labels` (`facade::write::resolve_priority`, which also accepts the canonical ordinal
key directly) and stored as that ordinal string; ranking is numeric on the ordinal;
`show`/`list`/`schema` map ordinal -> label via `cfg.priority.labels`; `--json` keeps the ordinal
(with additive `priority_label`/`type_label` fields for readers who want the display form).

NEVER store the label in the item record — it would make `--json` plugin-dependent.

THE GATE'S NAME, corrected 2026-09-18: this entry named `list_json_is_byte_identical_across_plugins`,
which no longer exists — a sentence that stays true while its pointer dies, exactly class 4 of
[sweep-the-deleted-vocabulary](2026-11-15-no-ticket-sweep-the-deleted-vocabulary.md). The live gate is
`list_json_is_plugin_independent_except_the_declared_type`
(`crates/cli/tests/guide_examples.rs`): it asserts the record is byte-identical across
`issue-tracker` and `personal-todo` once `type`, `priority_label` and `type_label` are removed —
the declared type being the one documented exception.

Related: `vocabulary.fields` gives plugin-named field headings for `show`/`schema`, with core
defaults in `default_field_label()`.
