# 2026-11-15 · no ticket · Splitting a function with #[cfg(target_os)] can make its callee dead code on the OTHER platform — use a cfg!() constant when you are only branching.

- Decided by: recorded in the project memory (`nxm`, key `cfg-split-kills-the-callee-elsewhere`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: rust, ci, platforms

## The record

Splitting a function into two versions with `#[cfg(target_os = ...)]` can KILL A DIFFERENT
function — on exactly the platform you are not building on.

MEASURED 2026-09-04 while fixing the launchd poisoning bug: `registration_line` was split into a
macOS version (which calls `describe_registration`) and a non-macOS version (which returns `None`).
That left `describe_registration` unreachable from anything outside macOS. `-D dead-code` (implied
by `-D warnings`) turned `quality-gates` (ubuntu) AND `Platform gates (windows-latest)` red — while
BOTH macOS lanes stayed green and `cargo clippy --all-targets` on the developer machine said
nothing.

WHY NOTHING SHOWS UP LOCALLY: on macOS the function is reachable, and `--all-targets` additionally
pulls in the `#[cfg(test)]` block, which uses it anyway. The fault exists only in the translation
nobody runs here.

THE FORM THAT AVOIDS IT: a `cfg!()` CONSTANT instead of a `#[cfg]` attribute.

    const LAUNCHD_LIVES_HERE: bool = cfg!(target_os = "macos");
    ...
    None => LAUNCHD_LIVES_HERE.then(|| NOT_CHECKED.to_string()),

The function then exists once, compiles everywhere, references its callee everywhere, and folds to
the same code. `#[cfg]` stays right where the BODY needs an API that does NOT EXIST on the other
platform (`plist_path_for`, `RealCtl` — there it MUST be split); for a pure branch over the
platform it is the wrong tool.

A cross-check before pushing is not possible on this machine: `cargo clippy --target
x86_64-unknown-linux-gnu` aborts on a C dependency ("failed to find tool x86_64-linux-gnu-gcc").
So CI really is the only gate here — consistent with [gates-run-in-ci-not-locally](2026-11-15-no-ticket-gates-run-in-ci-not-locally.md).
