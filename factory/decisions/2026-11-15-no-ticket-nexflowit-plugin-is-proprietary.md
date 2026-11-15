# 2026-11-15 · no ticket · The nexflow.it plugin is proprietary and must never live in this repo — the registry lets a consumer link its own without an OSS edit.

- Decided by: recorded in the project memory (`nxm`, key `nexflowit-plugin-is-proprietary`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: plugins, nexflowit, licensing

## The record

The nexflow.it note/person/goal plugin is **proprietary IP and must NOT live in this repository**
(nexus-flow is the open-source side, nexflow.it is not). It is defined in `nexflowit-app`.

THE SEAM THAT MAKES THAT POSSIBLE NOW EXISTS — verified in the code on 2026-09-05, and this is the
part that used to be an open requirement:
`crates/facade/src/plugin/registry.rs` is a compile-time self-registration registry. Every plugin
— the two bundled here (`issue-tracker`, `personal-todo`) and any consumer's proprietary one from
ITS OWN crate — `inventory::submit!`s one `PluginRegistration { name, toml, order }`, where `toml`
is that crate's own `include_str!`. `plugin::load`, `plugin::plugin_names` and `plugin::available`
read that roster exclusively, so a consumer plugin appears purely by registering and linking: no
OSS source change, and its TOML never lands in this repo or on disk. A workspace picks its plugin
by name (`[flow] plugin = "…"`); an unknown name is a validation error. `order` is declared, so
the listing is deterministic rather than link-order accident. The old hardcoded `PLUGIN_NAMES`
const is still exported but `#[deprecated]` since 0.16.0 as a facade coexistence window — it can
only ever name the two plugins compiled into THIS binary, so migrate any pin to `plugin_names()`.

The mechanism mirrors the module roster one level up (`nxs_init::module`), which is the same
inventory pattern for whole products.

STILL WORTH CHECKING WHENEVER THE MCP SURFACE GROWS: `nxs mcp serve` must not assume a consumer
workspace or a consumer plugin — a host that connects gets whatever plugin that workspace
declares, and nothing in the OSS tree may special-case one. A separate, still-parked idea is a
programmatic trait layer for plugins; it is NOT the same thing as this registry and should not be
confused with it.
