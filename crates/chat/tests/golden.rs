//! Golden-example harness for `nxc`'s contract verbs (mirrors `nxm`'s golden): every doc example is
//! executed against the built `nxc` binary and the shown output IS the asserted output (trycmd) —
//! documentation cannot drift without turning CI red.
//!
//! Determinism: cases run with `NXF_DETERMINISTIC_IDS=1` (fixed replica `ab12`, site 1) and a
//! pinned `NXC_NOW`, so init prefixes and minted channel/message ids are byte-stable.
//!
//! **`NXC_ACTOR` is set PER COMMAND, not here** (nxf 6j6v.t6vd). It used to be pinned on this
//! harness as `alice`, which was enough while every case had one speaker — but chat's own worked
//! example has two, and the loop it has to show is a persona ANSWERING a request. trycmd merges a
//! step's `$ KEY=VALUE cmd` prefix with the runner's env by `extend`ing the runner's over it
//! (`schema::Env::update`), so a harness-level `NXC_ACTOR` silently WINS over the per-command one:
//! `$ NXC_ACTOR=coder nxc reply …` posted as `alice` and the thread stayed outstanding, which is a
//! documentation example that quietly says the opposite of what it means. So the variable moved to
//! the command lines that care, where it is also visible to a reader — the handle a session mints
//! under is `<origin>/<NXC_ACTOR>`, and these goldens are what the guides quote.
//!
//! **`NXC_ORIGIN` is deliberately NOT pinned here** (nxf 6j6v.07me). The `<origin>` half of that
//! handle is the workspace's own replica prefix since the owner's decision of 2026-08-21, and this
//! harness is documentation: pinning an override would show every reader a default that no real
//! `nxc` produces. `NXF_DETERMINISTIC_IDS` is what makes it byte-stable, and it makes it stable at
//! the SAME `ab12` the `nxc init` case one block above prints as `prefix:` — the golden now shows
//! the minting and the handle it produces in one read, which it could not while the handle said
//! `local/`.
//!
//! Regenerate expected output after an intentional change with:
//!   TRYCMD=overwrite cargo test -p nexus-chat --test golden
//! That command does NOT rebuild `nxs` (the binary `nxc` symlinks to), so it is gated below —
//! see nexus-flow-0yyw.

#[test]
fn golden_examples() {
    // Before trycmd writes anything: under TRYCMD=overwrite a stale `nxs` would burn old output
    // into the goldens. See the flow golden harness for the full account.
    nxs_test_support::assert_multicall_binary_fresh();

    // Never this machine's own `$HOME` (nxf 6j6v.9bjv). A golden corpus asserts stdout AND stderr
    // byte for byte, so the service report every invocation makes when the host's program alias is
    // dangling (nxf 6j6v.dcpk) fails every case — and under `TRYCMD=overwrite` is burned into the
    // committed goldens, developer paths and all. See `pinned_home_env` for why each pair is the
    // value an unset variable resolves to anyway.
    let cases = trycmd::TestCases::new();
    for (key, value) in nxs_test_support::pinned_home_env() {
        cases.env(key, value);
    }
    cases
        .env("NXF_DETERMINISTIC_IDS", "1")
        // Never a real agent runtime from a test: every trigger is RECORDED instead of launching a
        // detached Claude Agent SDK session. `messaging.trycmd` sends to declared personas and to a
        // declared channel, and without this the golden corpus would spawn `node` (and pay for
        // tokens) on every CI run.
        .env("NXC_WORKER", "dry")
        // …and never a real SCHEDULER either, for the same reason one line up plus one this corpus
        // learned the hard way (nxf 6j6v.fabb): a golden that does not pin this is not deterministic
        // at all, it is a function of what the runner has installed. `messaging.trycmd` opens a
        // `working_tree: exclusive` channel and parks a commission behind it, which arms a drain —
        // and the arming BREADCRUMBS when the platform cannot schedule. So the case passed on macOS
        // (launchd is always there) and went red on the Linux runner (no `at` installed), from a
        // change that touched neither the verb nor its output.
        .env("NXC_TIMER", "dry")
        // …and never a real NAMER (nxf 6j6v.e76c), for the first of those two reasons exactly: the
        // shipped backend starts a detached run that asks a model what a conversation should be
        // called, so a corpus that did not pin this would spawn one — and pay for tokens — on every
        // `send` in every CI run. `dry` records the commission and derives the message's own first
        // words, so the whole path stays observable without a model.
        .env("NXC_NAMER", "dry")
        .env("NXC_NOW", "2026-06-20T10:00:00Z")
        .case("tests/golden/*.trycmd");
}
