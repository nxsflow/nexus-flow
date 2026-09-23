//! The OMISSION gate (nexus-flow-6j6v.vtvs): every CLI verb must have a counterpart on the
//! library seam an embedding app holds.
//!
//! The principle is specified once per module (E5 / E5m / E5c) and is the reason the facade layer
//! exists at all: **one shared core, with the CLI and an app sitting over it as equals** — an app
//! has the same capabilities as the command line, and the other way round. Each module already
//! carries a parity differential (`crates/{cli,memory,chat}/tests/parity.rs`) that drives the same
//! script through both seams and compares op logs and read results byte for byte.
//!
//! Those differentials are strong against DIVERGENCE and, by construction, blind to OMISSION: they
//! can only compare what exists on both sides. A verb that lives only in the CLI is invisible to
//! them. That is not hypothetical — `nxm prime` and `nxc prime` assembled their session-start text
//! as hard-wired `println!` prose inside `cli.rs` for a year with three green parity suites, until
//! app-foundations had to rebuild the same assembly in TypeScript (Foundation v0.35.0) because the
//! library could not hand it over. This gate is the other half: it compares the two verb LISTS, so
//! the next such omission fails at the seam instead of surfacing in another language.
//!
//! **Both lists are derived, never maintained.** The CLI list is walked out of the `clap` command
//! tree — the same tree that produces `--help` and the generated command reference — and the seam
//! list is parsed out of the module's own engine/facade sources. A gate whose input is a
//! hand-written second list drifts exactly like the thing it is watching.
//!
//! What the module has to declare is only the DIFFERENCE, and every entry carries a reason (see
//! [`Waiver`]). An exception without a reason is a finding, not a free pass.
//!
//! # What counts as the seam, and what deliberately does not
//!
//! The ticket names "the public method set of the respective `Engine`". Taken literally that is
//! narrower than the thing being protected, because the shared layer has two storeys: the long-lived
//! **handle** (`Engine`) and the **compute layer** under it (`read`/`write` for flow, `facade.rs`
//! for memory and chat). Both are public API an embedding app links — the epic's own audit of `nxm`
//! reads both ("neither `Engine` … nor `facade.rs` knows them") — so the seam here is their union.
//! Anything less would report `nxf recap` and `nxf schema` as omissions when the capability is
//! there and an app can call it.
//!
//! The union is not free, and the price is recorded rather than hidden: the handles do not
//! re-expose the foundation's `with_state`, so an app holding an `Engine` cannot aim the compute
//! layer at the store that handle owns — it has to open a second one. That is an asymmetry worth
//! closing, and it is filed as nexus-flow-6j6v.1ke9 rather than settled quietly here.
//!
//! What is emphatically NOT seam is the **store** beneath both. Reaching past the facade into the
//! store is the very thing E5c calls a violation — it is half the finding that opened epic
//! 6j6v.fjrc — so a capability that exists only there is an omission, not a counterpart. That
//! single choice is what makes `nxc transcript append` and `nxc session bind` come out of this
//! gate as findings instead of passing quietly. (`nxc agents` was the third of them until
//! 6j6v.dvyq §3 removed those verbs — a team is DECLARED, so there was never anything to lift, and
//! the waivers went with the verbs rather than with the debt.)
//!
//! # Scope: the three modules, not the umbrella
//!
//! `nxf`, `nxm` and `nxc` are gated; `nxs` deliberately is not. The umbrella owns no domain and has
//! no `Engine` — `init`, `prime`, `migrate`, `doctor`, `status`, `self-update` compose the modules
//! and act on the host — so every verb it has would enter as a `CliOnly` entry, and a table that is
//! all exception teaches a reader that exceptions are normal. If `nxs` ever grows a verb that acts
//! on the workspace rather than on the modules, that is the moment to gate it too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// Why a CLI verb has no same-named counterpart on the engine seam.
///
/// Each variant is a different claim about the world, and the gate holds each to as much proof as a
/// name comparison can carry — none of them is "shut up about this one":
///
/// * [`Waiver::CliOnly`] claims the verb *belongs* to the command line and an app would never call
///   it. The gate can only check the reason is there; a human checks it is true.
/// * [`Waiver::Alias`] claims the capability IS on the seam, under a different name. The gate
///   checks that a public symbol of that name EXISTS, so a rename or removal on the seam breaks the
///   waiver loudly instead of voiding it in silence. It cannot check the symbol does the same
///   thing — that is the review's job, and the reason every alias states which call the verb body
///   makes.
/// * [`Waiver::KnownGap`] claims nothing except that the gap is *known* and *tracked*. It requires
///   a board id, and it stops applying the moment the gap is closed.
///
/// **What the whole gate compares is names, not behaviour**, and that boundary is deliberate: a
/// coincidentally-named `pub fn` would satisfy it. Divergence between two implementations of the
/// same verb is the parity differential's job — it drives both seams and compares op logs byte for
/// byte. This one answers the question the differential structurally cannot ask: *is the verb there
/// at all?*
///
/// The `verb` of every variant is the space-joined CLI path exactly as a user types it after the
/// program name (`"dep add"`, `"transcript append"`) — so the table reads like the CLI it waives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiver {
    /// Deliberately command-line-own: the verb acts on the host (argv, the terminal, the user's
    /// config, the installed binary), not on the workspace, so there is nothing for the library
    /// seam to offer. `init`, `agent-manifest` and `self-update` are the epic-sanctioned shape.
    CliOnly {
        /// The space-joined CLI path (`"setup claude"`).
        verb: &'static str,
        /// Why this is the command line's own business. Required — an exception without a reason
        /// is a finding.
        reason: &'static str,
    },
    /// Present on the seam under a different name or shape (`nxf label list` → `read::labels`).
    /// The gate checks `seam` really is a public symbol on the seam, so the waiver cannot outlive
    /// what it points at.
    Alias {
        /// The space-joined CLI path (`"label list"`).
        verb: &'static str,
        /// The public seam symbol that carries this capability (`"labels"`).
        seam: &'static str,
        /// Why the names differ — a rename, a wider read, a projection of a bigger record.
        reason: &'static str,
    },
    /// A real omission that this change is not closing. Carries the board item that tracks it, so
    /// the debt is filed rather than absorbed: the difference between a gate and a reassurance.
    KnownGap {
        /// The space-joined CLI path (`"transcript append"`).
        verb: &'static str,
        /// The board item tracking the lift (`"6j6v.1234"`). Required.
        ticket: &'static str,
        /// What is missing on the seam, in one line.
        reason: &'static str,
    },
}

impl Waiver {
    /// The CLI path this waiver speaks for.
    fn verb(&self) -> &'static str {
        match self {
            Waiver::CliOnly { verb, .. }
            | Waiver::Alias { verb, .. }
            | Waiver::KnownGap { verb, .. } => verb,
        }
    }

    /// The reason text, whatever the variant calls it.
    fn reason(&self) -> &'static str {
        match self {
            Waiver::CliOnly { reason, .. }
            | Waiver::Alias { reason, .. }
            | Waiver::KnownGap { reason, .. } => reason,
        }
    }

    /// The waiver kind, for the failure text.
    fn kind(&self) -> &'static str {
        match self {
            Waiver::CliOnly { .. } => "CliOnly",
            Waiver::Alias { .. } => "Alias",
            Waiver::KnownGap { .. } => "KnownGap",
        }
    }
}

/// Fail the current test unless every verb in `cli` is answered by the seam parsed out of
/// `seam_sources` or by an entry in `waivers`.
///
/// `module` is the persona name used in the failure text (`"nxf"`). `seam_sources` are the module's
/// engine + facade compute-layer sources — the public API an embedding app links, as opposed to the
/// store underneath it (reaching past the facade into the store is the very thing E5c calls a
/// violation, so a symbol found only there is deliberately NOT seam).
pub fn assert_verb_seam(
    module: &str,
    cli: &clap::Command,
    seam_sources: &[PathBuf],
    waivers: &[Waiver],
) {
    let seam = match read_seam(seam_sources) {
        Ok(seam) => seam,
        Err(blind) => panic!("{}", blind_gate(module, &blind)),
    };
    if let Some(complaint) = verdict(module, &leaf_verbs(cli), &seam, waivers) {
        panic!("{complaint}");
    }
}

/// Every verb a user can actually invoke: the leaves of the `clap` tree, space-joined.
///
/// Two rules, both load-bearing:
///
/// * **Leaves only.** An intermediate node (`nxf dep`, `nxc transcript`) dispatches, it does not
///   act; its children are the verbs. A node that has subcommands AND runs bare is covered through
///   the child that spells out its default — `nxc channels`/`channels list` was the example until
///   6j6v.dvyq §3 removed the group, and `nxc workflow step`/`done` until the same item's last
///   block removed that one; `nxf dep`/`dep add` is the live one.
/// * **Hidden commands count.** `#[command(hide = true)]` hides a verb from `--help`, not from the
///   agent — `nxm prime`/`nxc prime` are hidden and are precisely the omission this gate exists
///   for. Only clap's own generated `help` subcommand is dropped, since it belongs to the parser.
pub fn leaf_verbs(cli: &clap::Command) -> Vec<String> {
    fn walk(cmd: &clap::Command, path: &[String], out: &mut Vec<String>) {
        let children: Vec<&clap::Command> = cmd
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
            .collect();
        if children.is_empty() {
            if !path.is_empty() {
                out.push(path.join(" "));
            }
            return;
        }
        for child in children {
            let mut next = path.to_vec();
            next.push(child.get_name().to_owned());
            walk(child, &next, out);
        }
    }
    let mut out = Vec::new();
    walk(cli, &[], &mut out);
    out.sort();
    out.dedup();
    out
}

/// The name a verb is expected to carry on the seam: path separators and kebab dashes both become
/// `_`, so `nxc transcript append` looks for `transcript_append` and `agent-manifest` for
/// `agent_manifest`. This mirrors what the code already does by hand — `Engine::transcript_page`
/// and `Engine::dep_add` are named after their verb paths — so the convention is discovered, not
/// imposed.
pub fn seam_name(verb: &str) -> String {
    verb.replace(['-', ' '], "_")
}

/// Parse the public symbol names off the seam sources.
///
/// Returns `Err(reason)` when the seam cannot be read at all — a moved or renamed source. That is
/// never a legitimate state: a gate that finds nothing to compare against reports "all clear"
/// forever, which is the exact silence it was built to end (same reasoning as the stale-binary
/// gate's `NoSources` verdict in [`crate`]).
pub fn read_seam(sources: &[PathBuf]) -> Result<BTreeSet<String>, String> {
    if sources.is_empty() {
        return Err("no seam sources were declared".to_owned());
    }
    let mut seam = BTreeSet::new();
    for src in sources {
        let text = std::fs::read_to_string(src)
            .map_err(|e| format!("{} could not be read: {e}", src.display()))?;
        let symbols = public_symbols(&text)
            .map_err(|e| format!("{} could not be parsed as Rust: {e}", src.display()))?;
        if symbols.is_empty() {
            return Err(format!(
                "{} declares no public functions at all",
                src.display()
            ));
        }
        seam.extend(symbols);
    }
    Ok(seam)
}

/// The public names an embedder can reach in one source file: free `pub fn`s, `pub fn` methods of
/// inherent `impl` blocks, `pub use` re-exports, and the same inside inline `pub mod`s.
///
/// **Trait impls are skipped**: a trait method is reachable only through the trait, is named by it,
/// and is never the seam a verb is lifted onto. A `#[cfg(test)] mod tests` is skipped for free by
/// the `pub mod` rule.
///
/// **`pub use` counts, because a lift can land as a re-export.** A verb moved into a submodule and
/// surfaced with `pub use inner::channel_open;` is on the seam every bit as much as one defined in
/// the file, and reading only `fn` definitions would report it as an omission — a gate that cries
/// wolf gets waivers written to silence it. The re-exported *name* is what is taken: the leaf of the
/// path, or the rename in `as`.
///
/// The one shape this cannot see is a **glob** (`pub use inner::*;`), which names nothing to read.
/// That is left as a known limitation rather than papered over, and it errs in the safe direction:
/// the gate under-reports the seam, so a glob-exported verb shows up as a finding to be looked at,
/// never as false coverage.
pub fn public_symbols(src: &str) -> Result<BTreeSet<String>, syn::Error> {
    fn is_public(vis: &syn::Visibility) -> bool {
        matches!(vis, syn::Visibility::Public(_))
    }
    /// Every name a `use` tree binds, following braces and honouring `as` renames.
    fn use_names(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(p) => use_names(&p.tree, out),
            syn::UseTree::Name(n) => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) => {
                out.insert(r.rename.to_string());
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| use_names(t, out)),
            // `pub use inner::*` — the names live in the other file; see the doc note above.
            syn::UseTree::Glob(_) => {}
        }
    }
    fn collect(items: &[syn::Item], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                syn::Item::Fn(f) if is_public(&f.vis) => {
                    out.insert(f.sig.ident.to_string());
                }
                syn::Item::Impl(imp) if imp.trait_.is_none() => {
                    for member in &imp.items {
                        if let syn::ImplItem::Fn(f) = member {
                            if is_public(&f.vis) {
                                out.insert(f.sig.ident.to_string());
                            }
                        }
                    }
                }
                syn::Item::Use(u) if is_public(&u.vis) => use_names(&u.tree, out),
                syn::Item::Mod(m) if is_public(&m.vis) => {
                    if let Some((_, items)) = &m.content {
                        collect(items, out);
                    }
                }
                _ => {}
            }
        }
    }
    let file = syn::parse_file(src)?;
    let mut out = BTreeSet::new();
    collect(&file.items, &mut out);
    Ok(out)
}

/// A function in one source file, together with the calls out of its BODY that a caller asked about.
///
/// Returned by [`method_callers`], which exists for gates that have to tell what a method DOES from
/// its source rather than from its name — the shape [`public_symbols`] deliberately cannot see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodCaller {
    /// The function's own name.
    pub name: String,
    /// Whether an embedder can reach it — `pub`, by the same rule [`public_symbols`] applies.
    /// Always `false` for a body that belongs to a trait, which carries no visibility of its own;
    /// ask [`trait_impl`](Self::trait_impl) rather than reading a `false` here as "private".
    pub public: bool,
    /// Whether the body belongs to a TRAIT rather than to file scope or an inherent `impl` — a
    /// method of an `impl Trait for T`, or a trait's own DEFAULT body (PR #430 review, Integrity
    /// #1). The two are one flag because every caller wants them on the same side of the line:
    /// neither is an inherent method an embedder reaches by that name, and a default body is as
    /// much "the trait's own code" as a delegator's. Reported instead of skipped, so a caller can
    /// decide — see [`method_callers`]'s doc for why this one is not filtered out at the source.
    pub trait_impl: bool,
    /// Whether the body is a METHOD — an item of an `impl` block, inherent or trait, or a trait's
    /// own default body — rather than a function at module scope (nxf 6j6v.b9nf).
    ///
    /// [`trait_impl`](Self::trait_impl) is a subset of it and answers a different question: that
    /// one is about whose code the body is, this one is about HOW IT CAN BE CALLED. It exists
    /// because Rust's method-call syntax resolves to methods and never to a function at module
    /// scope, which is what lets [`callers_reaching`] refuse an edge the language cannot take.
    pub is_method: bool,
    /// Which of the asked-for names its body reached, sorted. Both spellings of a call are in
    /// here — see [`called_by_path`](Self::called_by_path) for the split.
    pub calls: BTreeSet<String>,
    /// The subset of [`calls`](Self::calls) this body reached with PATH syntax at least once —
    /// `send(x)` or `Type::send(&t, x)` — as opposed to only with method syntax, `t.send(x)`
    /// (nxf 6j6v.b9nf).
    ///
    /// A name in `calls` and not in here was ONLY ever spelled as a method call, which is the one
    /// case in which [`callers_reaching`] may drop the edge: see its doc for the rule and for the
    /// false finding that bought it.
    pub called_by_path: BTreeSet<String>,
}

/// Every function in `src` — free, or a method of an inherent `impl` — whose body CALLS at least one
/// of `methods`, with its visibility and which ones it reached.
///
/// **A syntax walk over call expressions, not a text search**, for the reason
/// [`crate::actor_contract`]'s detector gives and for a measured one: `chat`'s `engine.rs` names
/// `Engine::with_orchestration` inside the doc comment of `resolve_definitions`, which writes
/// nothing at all. A gate that grepped would report that method as a writer and teach its readers
/// that its findings need checking by hand.
///
/// **Both spellings of a call are matched** — `x.funnel(…)` and `Type::funnel(&x, …)` — because a
/// gate that knows one spelling of the thing it forbids is exactly the hole it was built to close.
/// A path call matches on its LAST segment, so `Handle::with_state_mut` counts and a same-named
/// method on an unrelated type counts too: this errs toward "look at this", which is the safe
/// direction for a gate whose finding is a sentence to write, not a build to fix. Which spelling
/// each one was is kept ([`MethodCaller::called_by_path`]); this single-level view does not act on
/// it, and [`callers_reaching`] does, for a reason that only arises once files are joined.
///
/// **A call to the function's own PARAMETER is not a call to the function it is named after**, and
/// that one IS acted on here, because it is decidable inside the one body: see [`call_graph`],
/// where the rule and the false finding that bought it are written down (nxf 6j6v.b9nf).
///
/// **Bodies belonging to a trait are REPORTED, not skipped** — unlike [`public_symbols`], which
/// drops them because a trait method is never the seam a verb is lifted onto. Here the question is
/// what a body DOES, and a body inside `impl SomeTrait for T` does it just as much as any other;
/// dropping them would leave a caller's gate blind to a write introduced that way, with nothing
/// saying so (PR #427 review, Code Quality #1 / Integrity #1 — found independently by two
/// reviewers). A trait's own DEFAULT body is the same shape and was invisible until PR #430's
/// review found it: it runs for every implementor that does not override it. Both come back with
/// [`MethodCaller::trait_impl`] set and `public: false`, because neither carries a visibility of
/// its own; a caller that wants only inherent methods filters on that field, and one that wants to
/// REFUSE such a body can see it.
///
/// **Macro-expanded call sites are the one blind spot that remains**, and it is named rather than
/// left to be discovered: [`syn::parse_file`] reads the source BEFORE expansion, so a call produced
/// by a macro body is invisible here. It errs in the under-reporting direction, which for a gate
/// whose finding is "somebody has to look at this" is the dangerous one — so a caller relying on
/// completeness should say so where it makes the claim.
///
/// Nested functions are not unwrapped: a call inside a closure or an inner `fn` belongs to the
/// enclosing item, which is the honest answer for "does this entry point reach the write path".
///
/// The walk itself lives in [`call_graph`], of which this is the filtered view: it drops every
/// function that reached none of `methods`, which is what makes it read as "the callers of X".
pub fn method_callers(src: &str, methods: &[&str]) -> Result<Vec<MethodCaller>, syn::Error> {
    Ok(call_graph(src)?
        .into_iter()
        .filter_map(|mut caller| {
            caller.calls.retain(|name| methods.contains(&name.as_str()));
            caller
                .called_by_path
                .retain(|name| methods.contains(&name.as_str()));
            (!caller.calls.is_empty()).then_some(caller)
        })
        .collect())
}

/// Every function in `src` with EVERY call its body makes, by the name at the call site — the whole
/// call graph of one file, which [`method_callers`] is one filtered view of.
///
/// It exists for the question a single-level detector cannot answer: **can this entrance reach that
/// funnel, through however many frames of somebody else's code?** `chat`'s write seam is four
/// one-line forwardings onto a compute layer thousands of lines deep, so "does `Engine::withdraw`
/// start a session" is a question about a path, not about a call (nxf 6j6v.12nn). Joining the
/// graphs of several files by NAME and walking the result answers it.
///
/// **A node is a NAME, not a function**, and joining files makes that consequential: `chat` has
/// both `Engine::withdraw` and `orchestration::withdraw`, and here they are one node whose calls
/// are the union of both bodies. For a seam of thin forwardings that is the RIGHT merge — the
/// forwarding and the verb it forwards onto are one path — but it errs toward reachable, which is
/// the safe direction for a gate whose finding is a sentence to write. A caller that cannot afford
/// that has to narrow the sources it joins, and say so.
///
/// **A name a body calls is not always a function this graph has.** Two spellings LOOK like a call
/// to a same-named function and cannot be one, and both were measured in `chat` rather than
/// imagined (nxf 6j6v.b9nf): `park::WorkingCopy::git_bytes` drains a child's pipes with
/// `tx.send(buf)` on an `mpsc` sender, and `worker::SidecarWorker::stop_session_by` takes its
/// signaller as `send: impl Fn(u32) -> …` and calls it `send(pid)`. `chat` also has a free
/// `fn send` — the facade's and the CLI's `nxc send` — which reaches the spawn funnel, and joined
/// by name both of those read as commissioning work. `chat/tests/read_surface.rs` duly reported
/// that `Engine::withdraw` starts sessions, which it does not.
///
/// So the two are handled apart, and the one that is decidable inside a single body is refused
/// here:
///
/// * **A bare call to a name the enclosing function takes as a PARAMETER is a call to that
///   parameter**, and no edge is recorded. A parameter is in scope for the whole body and shadows
///   a function of that name in the value namespace, so this removes an edge the language cannot
///   take rather than one this gate would rather not see. `let` bindings are deliberately NOT
///   treated this way — a `let` shadows only from its own statement onward, so a call above it
///   really is the function.
/// * **Which spelling each call used is recorded** ([`MethodCaller::called_by_path`]) and acted on
///   one level up, in [`callers_reaching`], because whether a name is a method or a module-scope
///   function is a fact about the JOINED graph and not about one file.
///
/// Every other property [`method_callers`] documents holds here unchanged, because it is the same
/// walk: both spellings of a call, path calls matched on their last segment, bodies belonging to a
/// trait — an `impl Trait for T` method and a trait's own default body alike — reported with
/// [`MethodCaller::trait_impl`] set, `#[cfg(test)]` modules skipped, macro-expanded call sites
/// invisible. Unlike that view this one keeps functions whose body calls NOTHING: a leaf
/// is a node of the graph, and dropping it would make "reaches nothing" indistinguishable from "is
/// not there at all".
pub fn call_graph(src: &str) -> Result<Vec<MethodCaller>, syn::Error> {
    use syn::visit::Visit;

    #[derive(Default)]
    struct Calls {
        found: BTreeSet<String>,
        by_path: BTreeSet<String>,
        /// The enclosing function's own PARAMETER names. A parameter is in scope for the whole
        /// body and shadows a function of that name in the value namespace, so a bare `send(pid)`
        /// under `fn f(send: impl Fn(u32))` calls the parameter and cannot be the free `fn send`
        /// (nxf 6j6v.b9nf). Recording it as one invents a path through a function the program
        /// never enters, which is exactly what happened to `chat`'s write-side gate.
        ///
        /// Parameters ONLY, deliberately: a `let` of the same name shadows from its own statement
        /// onward, so a call above it really is the function, and dropping that edge would err
        /// toward UNREACHABLE — the direction this graph must never take.
        shadowed: BTreeSet<String>,
    }
    impl<'ast> Visit<'ast> for Calls {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            self.found.insert(call.method.to_string());
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*call.func {
                let bare = p.qself.is_none() && p.path.segments.len() == 1;
                if let Some(last) = p.path.segments.last() {
                    let name = last.ident.to_string();
                    if !(bare && self.shadowed.contains(&name)) {
                        self.found.insert(name.clone());
                        self.by_path.insert(name);
                    }
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
    }

    /// The parameter names that shadow a function of the same name inside this body — simple
    /// `ident: Type` bindings, which is the shape an injected collaborator takes. A destructuring
    /// pattern binds no name a call could be spelled with and is skipped.
    fn parameters(sig: &syn::Signature) -> BTreeSet<String> {
        sig.inputs
            .iter()
            .filter_map(|arg| match arg {
                syn::FnArg::Typed(t) => match &*t.pat {
                    syn::Pat::Ident(id) => Some(id.ident.to_string()),
                    _ => None,
                },
                syn::FnArg::Receiver(_) => None,
            })
            .collect()
    }

    fn reached(block: &syn::Block, sig: &syn::Signature) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut calls = Calls {
            shadowed: parameters(sig),
            ..Calls::default()
        };
        calls.visit_block(block);
        (calls.found, calls.by_path)
    }

    fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|a| match &a.meta {
            syn::Meta::List(l) => l.path.is_ident("cfg") && l.tokens.to_string().trim() == "test",
            _ => false,
        })
    }

    fn collect(items: &[syn::Item], out: &mut Vec<MethodCaller>) {
        fn is_public(vis: &syn::Visibility) -> bool {
            matches!(vis, syn::Visibility::Public(_))
        }
        for item in items {
            match item {
                syn::Item::Fn(f) => {
                    let (calls, called_by_path) = reached(&f.block, &f.sig);
                    out.push(MethodCaller {
                        name: f.sig.ident.to_string(),
                        public: is_public(&f.vis),
                        trait_impl: false,
                        is_method: false,
                        calls,
                        called_by_path,
                    });
                }
                syn::Item::Impl(imp) => {
                    let trait_impl = imp.trait_.is_some();
                    for member in &imp.items {
                        if let syn::ImplItem::Fn(f) = member {
                            let (calls, called_by_path) = reached(&f.block, &f.sig);
                            out.push(MethodCaller {
                                name: f.sig.ident.to_string(),
                                public: !trait_impl && is_public(&f.vis),
                                trait_impl,
                                is_method: true,
                                calls,
                                called_by_path,
                            });
                        }
                    }
                }
                // A TRAIT's own DEFAULT bodies, walked exactly the way an `impl`'s are (PR #430
                // review, Integrity #1). A default body runs for every implementor that does not
                // override it, so it is a body that does things — and until this it was in no
                // graph at all, because `syn::Item::Trait` fell into the catch-all below. That
                // was the same hole the callers of this function exist to close, one level up:
                // a default method reaching a watched funnel would have left every gate green
                // with nothing to notice. Signature-only items (`fn f(&self);`) have no body and
                // are not nodes.
                syn::Item::Trait(tr) => {
                    for member in &tr.items {
                        if let syn::TraitItem::Fn(f) = member {
                            if let Some(block) = &f.default {
                                let (calls, called_by_path) = reached(block, &f.sig);
                                out.push(MethodCaller {
                                    name: f.sig.ident.to_string(),
                                    public: false,
                                    trait_impl: true,
                                    is_method: true,
                                    calls,
                                    called_by_path,
                                });
                            }
                        }
                    }
                }
                // Private modules ARE followed — a funnel can hide in one — but a
                // `#[cfg(test)] mod tests` is not: its helpers are not the surface, and a test
                // fixture that calls the watched name would read as a finding.
                // `public_symbols` skips them for free through its `pub mod` rule; this one has
                // to say so.
                syn::Item::Mod(m) if !is_cfg_test(&m.attrs) => {
                    if let Some((_, items)) = &m.content {
                        collect(items, out);
                    }
                }
                _ => {}
            }
        }
    }

    let file = syn::parse_file(src)?;
    let mut out = Vec::new();
    collect(&file.items, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Every function in `graph` that can reach one of `targets` — directly, or through any chain of
/// calls the graph records.
///
/// The companion to [`call_graph`], and the question a per-call detector cannot answer: `chat`'s
/// write seam is a handful of one-line forwardings onto a compute layer thousands of lines deep, so
/// "can this call start a session" is about a PATH rather than about the forwarding itself (nxf
/// 6j6v.12nn) — measured against the source, not read off which write it happens to be. (An earlier
/// version of this doc named `Engine::release_working_tree` as the worked example; that call left
/// this crate's own seam with the verb it forwarded to, nxf 6j6v.b9nf.)
///
/// **The targets themselves are not in the answer** — a funnel does not reach itself — and a cycle
/// terminates rather than spinning, because a name enters the set once.
///
/// It inherits every approximation of the graph it is given, and the direction matters: a node is a
/// NAME, so joining files merges same-named functions and the answer errs toward REACHABLE. For a
/// gate that reads "this row says it starts nothing and the source disagrees", erring that way
/// means a false FINDING rather than a false all-clear — the safe direction, and the caller should
/// still say which sources it joined and why.
///
/// **One edge of that merge is refused, and it is refused because RUST cannot take it** (nxf
/// 6j6v.b9nf): `t.send(x)` never resolves to a `fn send` at module scope. So an edge whose callee
/// this body only ever spelled as a METHOD CALL is dropped when the graph defines that name at
/// module scope and defines no method of that name for it to have meant. A name the graph does not
/// define at all keeps its edge — it may be the funnel being looked for.
///
/// That is a sharpening and not a waiver, and the difference is the whole of why it is done here
/// rather than by excusing a row: it removes paths the program cannot run, and leaves every path
/// it can. The false finding it was built out of is written up on [`call_graph`], together with
/// the second shape — a call to the function's own parameter — which is decidable one level down
/// and is dropped there.
pub fn callers_reaching(graph: &[MethodCaller], targets: &[&str]) -> BTreeSet<String> {
    // The names this graph defines as METHODS and the names it defines at module scope — the two
    // halves of the one edge this walk refuses (nxf 6j6v.b9nf, see the doc above).
    let mut a_method: BTreeSet<&str> = BTreeSet::new();
    let mut a_free_function: BTreeSet<&str> = BTreeSet::new();
    for f in graph {
        if f.is_method {
            a_method.insert(f.name.as_str());
        } else {
            a_free_function.insert(f.name.as_str());
        }
    }
    let mut callers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in graph {
        for callee in &f.calls {
            let name = callee.as_str();
            // **`t.send(x)` is not a call to `fn send`.** Dropped only when all three hold: this
            // body never spelled the name as a path, the graph defines it at module scope, and
            // the graph defines no method of that name for the call to have meant. A name the
            // graph does not define at all keeps its edge — it may be the funnel itself.
            if !f.called_by_path.contains(name)
                && a_free_function.contains(name)
                && !a_method.contains(name)
            {
                continue;
            }
            callers.entry(name).or_default().insert(&f.name);
        }
    }
    let mut reaching = BTreeSet::new();
    let mut frontier: Vec<&str> = targets.to_vec();
    while let Some(callee) = frontier.pop() {
        let Some(direct) = callers.get(callee) else {
            continue;
        };
        for caller in direct {
            if reaching.insert((*caller).to_owned()) {
                frontier.push(caller);
            }
        }
    }
    reaching
}

/// Every `pub fn` in `src` whose SIGNATURE lends out a `&mut <type_name>` — the entrances through
/// which a caller can be handed mutable access to that type.
///
/// It exists so a gate that claims to know every route to a mutable store can hold that claim at
/// the ROOT instead of asserting it (PR #427 review, Test Quality #2). A gate keyed on the names of
/// the wrappers it knows cannot see a wholly new wrapper; a gate on the thing being wrapped can —
/// the day the handle grows a second lender, the list of names has to be re-decided.
///
/// A syntax walk, so it sees the shape wherever it sits: a plain argument, a return type, or —
/// the case this was written for — nested inside an `impl FnOnce(&mut State<S>) -> …` bound.
/// Matching is on the LAST path segment of the referent, so `foundation::State` and `State` are the
/// same answer, and a same-named type from elsewhere is a false positive rather than a miss.
///
/// Trait impls are skipped here, as in [`public_symbols`]: the question is which entrances a caller
/// can reach on the type itself.
pub fn public_fns_lending_mut(src: &str, type_name: &str) -> Result<BTreeSet<String>, syn::Error> {
    use syn::visit::Visit;

    struct Lends<'a> {
        wanted: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Lends<'_> {
        fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
            if r.mutability.is_some() {
                if let syn::Type::Path(p) = &*r.elem {
                    if p.path
                        .segments
                        .last()
                        .is_some_and(|seg| seg.ident == self.wanted)
                    {
                        self.found = true;
                    }
                }
            }
            syn::visit::visit_type_reference(self, r);
        }
    }

    fn lends(sig: &syn::Signature, type_name: &str) -> bool {
        let mut v = Lends {
            wanted: type_name,
            found: false,
        };
        v.visit_signature(sig);
        v.found
    }

    fn collect(items: &[syn::Item], type_name: &str, out: &mut BTreeSet<String>) {
        let is_public = |vis: &syn::Visibility| matches!(vis, syn::Visibility::Public(_));
        for item in items {
            match item {
                syn::Item::Fn(f) if is_public(&f.vis) && lends(&f.sig, type_name) => {
                    out.insert(f.sig.ident.to_string());
                }
                syn::Item::Impl(imp) if imp.trait_.is_none() => {
                    for member in &imp.items {
                        if let syn::ImplItem::Fn(f) = member {
                            if is_public(&f.vis) && lends(&f.sig, type_name) {
                                out.insert(f.sig.ident.to_string());
                            }
                        }
                    }
                }
                syn::Item::Mod(m) if is_public(&m.vis) => {
                    if let Some((_, items)) = &m.content {
                        collect(items, type_name, out);
                    }
                }
                _ => {}
            }
        }
    }

    let file = syn::parse_file(src)?;
    let mut out = BTreeSet::new();
    collect(&file.items, type_name, &mut out);
    Ok(out)
}

/// The whole decision, as a pure function over the two derived lists and the declared difference —
/// so every failure mode is unit-testable without a workspace, a binary, or a clap tree.
///
/// `None` means the seam holds. `Some(text)` is the complaint, already formatted for the panic.
fn verdict(
    module: &str,
    verbs: &[String],
    seam: &BTreeSet<String>,
    waivers: &[Waiver],
) -> Option<String> {
    let known: BTreeSet<&str> = verbs.iter().map(String::as_str).collect();
    let mut by_verb: BTreeMap<&str, Vec<&Waiver>> = BTreeMap::new();
    for w in waivers {
        by_verb.entry(w.verb()).or_default().push(w);
    }

    // Findings about the WAIVER TABLE itself. A waiver that no longer describes reality is worse
    // than no waiver: it reads as a considered decision while covering for nothing.
    let mut table: Vec<String> = Vec::new();
    for (verb, entries) in &by_verb {
        if entries.len() > 1 {
            table.push(format!(
                "{verb:<24} listed {} times — one verb, one reason.",
                entries.len()
            ));
        }
        for w in entries {
            if w.reason().trim().is_empty() {
                table.push(format!(
                    "{verb:<24} {} carries no reason. An exception without a reason is a \
                     finding, not a free pass.",
                    w.kind()
                ));
            }
            if let Waiver::KnownGap { ticket, .. } = w {
                if ticket.trim().is_empty() {
                    table.push(format!(
                        "{verb:<24} KnownGap names no board item. A gap without a ticket is not \
                         a waiver — file it, then name it here."
                    ));
                }
            }
            if let Waiver::Alias { seam: target, .. } = w {
                if !seam.contains(*target) {
                    table.push(format!(
                        "{verb:<24} Alias points at `{target}`, which is not on the seam \
                         (renamed? removed?). The alias is void — re-point or re-file it.",
                    ));
                }
            }
        }
        if !known.contains(verb) {
            table.push(format!(
                "{verb:<24} is waived but is no longer a {module} verb — drop the entry."
            ));
        } else if seam.contains(seam_name(verb).as_str()) {
            table.push(format!(
                "{verb:<24} is waived but IS on the seam now as `{}` — the debt is paid, drop \
                 the entry.",
                seam_name(verb)
            ));
        }
    }

    // The finding this gate exists for: a verb the command line can do and the library cannot,
    // with nobody having said why.
    let uncovered: Vec<&String> = verbs
        .iter()
        .filter(|v| !seam.contains(seam_name(v).as_str()) && !by_verb.contains_key(v.as_str()))
        .collect();

    if uncovered.is_empty() && table.is_empty() {
        return None;
    }

    // Two different failures wear this gate's name, and conflating them sends the reader looking
    // for the wrong thing: a verb the library genuinely cannot do, versus a waiver that has stopped
    // being true. Name whichever one actually happened.
    let headline = if uncovered.is_empty() {
        "the waiver table no longer matches reality."
    } else {
        "the command line can do something the library cannot."
    };
    let mut out = format!("\n\nVERB SEAM PARITY ({module}) — {headline}\n");
    if !uncovered.is_empty() {
        out.push_str(
            "\n  Verbs with no counterpart on the engine seam, and no recorded reason:\n\n",
        );
        for v in &uncovered {
            out.push_str(&format!(
                "    {module} {v:<28} (looked for `{}`)\n",
                seam_name(v)
            ));
        }
    }
    if !table.is_empty() {
        out.push_str("\n  Waivers that no longer hold:\n\n");
        for f in &table {
            out.push_str(&format!("    {f}\n"));
        }
    }
    if !uncovered.is_empty() {
        out.push_str(&format!(
            "\n  One shared core, with the CLI and an app over it as equals (E5/E5m/E5c, epic\n  \
             6j6v.fjrc): every verb an agent can type must be reachable by an app holding the\n  \
             library handle. The parity differential next door compares what exists on BOTH seams\n  \
             and is blind to a verb that exists on only one — this gate is that other half.\n\n  \
             Close it in this order of preference:\n\n    \
             1. LIFT the verb's body into the facade/`Engine` and let the CLI render the result.\n       \
             That is what `{module} prime` went through in 6j6v.fjrc, and it is why an app now\n       \
             gets the same bytes instead of rebuilding them.\n    \
             2. If the verb is genuinely the command line's own (host config, argv, the installed\n       \
             binary), add a `Waiver::CliOnly` WITH its reason.\n    \
             3. If the capability is on the seam under another name, add a `Waiver::Alias` naming\n       \
             it — the gate checks the symbol exists.\n    \
             4. If it is a real gap you are not closing now, FILE a board item and add a\n       \
             `Waiver::KnownGap` naming it. Filing it is the difference between a gate and a\n       \
             reassurance.\n"
        ));
    }
    out.push_str(
        "\n  The table lives in this module's `tests/verb_seam.rs`. See nexus-flow-6j6v.vtvs.\n",
    );
    Some(out)
}

/// The gate has lost sight of the seam it watches. Distinct from a normal failure, and phrased so
/// nobody goes looking for a missing verb: there is nothing to compare, so every verdict from here
/// on would be a false all-clear.
fn blind_gate(module: &str, detail: &str) -> String {
    format!(
        "\n\nVERB SEAM GATE IS BLIND ({module}) — it cannot read the seam it compares against.\n\n  \
         {detail}\n\n\
         This is NOT a missing verb. The gate parses the module's engine + facade sources to\n\
         derive the seam; with none of them readable it would report `all clear` forever, which\n\
         is exactly the silence nexus-flow-6j6v.vtvs exists to abolish.\n\n\
         Something moved — a renamed file, a relocated crate. Re-point the source list in this\n\
         module's `tests/verb_seam.rs`; do not delete the gate.\n"
    )
}

/// Resolve a path relative to the calling crate's `CARGO_MANIFEST_DIR`, so a seam source list reads
/// as the relative path a human would type (`"../facade/src/read.rs"`) and still works from any
/// working directory.
///
/// # Panics
///
/// If `rel` is absolute. `PathBuf::push` would silently DISCARD the manifest prefix and resolve
/// somewhere else entirely — most likely onto a file that does not exist, which this gate would then
/// report as a blind seam and send someone hunting a moved crate. A loud panic naming the argument
/// is the whole point: a gate that fails obscurely gets deleted rather than fixed.
pub fn crate_relative(manifest_dir: &str, rel: &str) -> PathBuf {
    let mut out = PathBuf::from(manifest_dir);
    for component in Path::new(rel).components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => panic!(
                "verb-seam source `{rel}` is an ABSOLUTE path; it must be relative to the crate \
                 (e.g. `src/engine.rs` or `../facade/src/read.rs`), or it silently resolves away \
                 from `{manifest_dir}`"
            ),
            c => out.push(c.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim [`method_callers`]'s doc makes about why it walks the AST: a name in a doc
    /// comment is not a call. `chat`'s `engine.rs` has exactly this shape — `resolve_definitions`
    /// names `Engine::with_orchestration` in its doc and writes nothing — and a gate that counted
    /// it would report a read as a write.
    #[test]
    fn a_funnel_named_in_a_doc_comment_is_not_a_call() {
        let src =
            "impl E {\n    /// See [`E::with_state_mut`] for the lock this does NOT take.\n    \
                   fn resolve(&self) -> u8 { 0 }\n}\n";
        let found = method_callers(src, &["with_state_mut"]).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    /// Both spellings of the same call, because a detector that knows one is the hole it was built
    /// to close.
    #[test]
    fn a_call_is_found_as_a_method_and_as_a_path() {
        let src = "impl E {\n    \
                   pub fn a(&self) { self.handle.with_state_mut(|s| s); }\n    \
                   fn b(&self) { Handle::with_state_mut(&self.handle, |s| s); }\n    \
                   pub fn c(&self) { self.read(); }\n}\n";
        let found = method_callers(src, &["with_state_mut"]).unwrap();
        let names: Vec<(&str, bool)> = found.iter().map(|f| (f.name.as_str(), f.public)).collect();
        assert_eq!(names, vec![("a", true), ("b", false)], "{found:?}");
    }

    /// A `#[cfg(test)] mod` is not the surface: a fixture that calls the watched name would read
    /// as a finding, and `engine.rs` would grow one the day anybody adds a unit test to it.
    #[test]
    fn a_call_inside_a_cfg_test_module_is_not_the_surface() {
        let src =
            "#[cfg(test)]\nmod tests {\n    fn helper() { thing.with_state_mut(|s| s); }\n}\n\
                   mod inner { pub fn real() { thing.with_state_mut(|s| s); } }\n";
        let found = method_callers(src, &["with_state_mut"]).unwrap();
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["real"], "{found:?}");
    }

    /// A trait-impl body is REPORTED rather than dropped — the gap PR #427's review found twice.
    /// `public` is `false` there because such a method has no visibility of its own, so a caller
    /// must read `trait_impl` and not mistake it for a private helper.
    #[test]
    fn a_call_inside_a_trait_impl_is_reported_and_flagged() {
        let src = "impl E { pub fn a(&self) { self.h.with_state_mut(|s| s); } }\n\
                   impl SomeTrait for E { fn t(&self) { self.h.with_state_mut(|s| s); } }\n";
        let found = method_callers(src, &["with_state_mut"]).unwrap();
        let rows: Vec<(&str, bool, bool)> = found
            .iter()
            .map(|f| (f.name.as_str(), f.public, f.trait_impl))
            .collect();
        assert_eq!(
            rows,
            vec![("a", true, false), ("t", false, true)],
            "{found:?}"
        );
    }

    /// A trait's own DEFAULT body is a body, and it was in no graph at all until PR #430's review
    /// found it (Integrity #1): every implementor that does not override it runs this code, so a
    /// default method reaching a watched funnel is exactly the shape these gates exist to catch.
    /// A signature with no body is not a node — there is nothing to walk.
    #[test]
    fn a_default_body_on_a_trait_is_a_node_and_a_bare_signature_is_not() {
        let src = "pub trait Worker {\n    \
                   fn trigger(&self, r: R) -> T;\n    \
                   fn answers(&self) -> bool { self.h.with_state_mut(|s| s) }\n}\n\
                   impl Worker for W { fn trigger(&self, r: R) -> T { forward(r) } }\n";
        let graph = call_graph(src).unwrap();
        let rows: Vec<(&str, bool, bool)> = graph
            .iter()
            .map(|f| (f.name.as_str(), f.public, f.trait_impl))
            .collect();
        assert_eq!(
            rows,
            vec![("answers", false, true), ("trigger", false, true)],
            "the bare signature must not be a node and the default body must be one: {rows:?}"
        );
        let found = method_callers(src, &["with_state_mut"]).unwrap();
        assert_eq!(
            found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            vec!["answers"],
            "a call made from a default body has to be visible to the filtered view too: {found:?}"
        );
    }

    /// [`call_graph`] keeps what [`method_callers`] drops: a function that calls nothing is a NODE
    /// of the graph, and losing it would make "reaches nothing" and "is not there" the same answer.
    #[test]
    fn the_call_graph_keeps_a_leaf_that_the_filtered_view_drops() {
        let src = "impl E {\n    \
                   pub fn a(&self) { self.h.with_state_mut(|s| s); self.b(); }\n    \
                   fn leaf(&self) -> u8 { 0 }\n}\n";
        let graph = call_graph(src).unwrap();
        let rows: Vec<(&str, Vec<&str>)> = graph
            .iter()
            .map(|f| {
                (
                    f.name.as_str(),
                    f.calls.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![("a", vec!["b", "with_state_mut"]), ("leaf", vec![])],
            "{graph:?}"
        );
        assert!(method_callers(src, &["with_state_mut"])
            .unwrap()
            .iter()
            .all(|f| f.name != "leaf"));
    }

    /// The whole point of [`callers_reaching`]: the entrance does not call the funnel, it calls
    /// something that does. And a cycle terminates rather than spinning — `chat`'s orchestration
    /// has several.
    #[test]
    fn reaching_walks_through_a_middleman_and_survives_a_cycle() {
        let src = "impl E {\n    \
                   pub fn entrance(&self) { middle(); }\n    \
                   pub fn loops(&self) { partner(); funnel(); }\n    \
                   pub fn partner(&self) { loops(); }\n    \
                   pub fn unrelated(&self) { something_else(); }\n}\n\
                   fn middle() { funnel(); }\n";
        let graph = call_graph(src).unwrap();
        let reaching = callers_reaching(&graph, &["funnel"]);
        assert_eq!(
            reaching.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["entrance", "loops", "middle", "partner"],
            "{reaching:?}"
        );
    }

    /// A funnel does not reach itself, and a target nobody calls is not a silent all-clear — it is
    /// an empty answer the caller has to notice for itself.
    #[test]
    fn reaching_excludes_the_target_and_answers_empty_for_an_uncalled_one() {
        let src = "fn funnel() { funnel_inner(); }\nfn caller() { funnel(); }\n";
        let graph = call_graph(src).unwrap();
        assert_eq!(
            callers_reaching(&graph, &["funnel"])
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["caller"]
        );
        assert!(callers_reaching(&graph, &["nobody_calls_this"]).is_empty());
    }

    /// **`x.send(…)` is not a call to `fn send`, and a graph that merges the two invents a path
    /// through a function the program never enters** (nxf 6j6v.b9nf).
    ///
    /// The measured shape: `chat`'s `park::WorkingCopy::git_bytes` drains a child's pipes over an
    /// `mpsc` channel and writes `tx.send(buf)`, while `chat` also has a free `fn send` — the CLI's
    /// and the facade's `nxc send` — which reaches the spawn funnel. Joined by name, an `mpsc`
    /// handoff inside a git runner read as the verb that commissions work, and the write-side gate
    /// in `chat/tests/read_surface.rs` reported `withdraw` as a call that starts sessions.
    ///
    /// The rule is a SOUND one rather than a heuristic, which is why it is applied instead of
    /// waived: Rust's method-call syntax resolves to inherent and trait methods, and never to a
    /// function at module scope. So the edge is one the language cannot take, and dropping it
    /// removes no real path. Where the name IS a method somewhere in the graph the edge stays —
    /// the second half below — because then the by-name join is doing the job it exists for.
    #[test]
    fn a_method_call_does_not_reach_a_free_function_of_the_same_name() {
        let src = "fn entrance(tx: T, buf: B) { let _ = tx.send(buf); }\n\
                   fn send(req: R) { funnel(req); }\n";
        let graph = call_graph(src).unwrap();
        let reaching = callers_reaching(&graph, &["funnel"]);
        assert_eq!(
            reaching.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["send"],
            "a channel handoff spelled `tx.send(..)` was read as a call to `fn send`: {reaching:?}"
        );

        // The other direction, twice, because a rule that dropped every edge of that name would be
        // a blindfold rather than a sharpening: a PATH call to the free function still reaches it,
        // and a method call whose name IS defined as a method still reaches that.
        let by_path = "fn entrance(req: R) { send(req); }\nfn send(req: R) { funnel(req); }\n";
        assert!(
            callers_reaching(&call_graph(by_path).unwrap(), &["funnel"]).contains("entrance"),
            "a genuine call to the free function stopped being a path"
        );
        let on_a_method = "impl W { fn send(&self, req: R) { funnel(req); } }\n\
                           fn entrance(w: W, req: R) { w.send(req); }\n";
        assert!(
            callers_reaching(&call_graph(on_a_method).unwrap(), &["funnel"]).contains("entrance"),
            "a method call to a name the graph defines as a METHOD stopped being a path"
        );
    }

    /// **A call to a PARAMETER is a call to that parameter, not to the function it is named after**
    /// (nxf 6j6v.b9nf) — the second edge the same false finding was built out of.
    ///
    /// The measured shape: `chat`'s `worker::SidecarWorker::stop_session_by` takes the signaller as
    /// `send: impl Fn(u32) -> …`, so a test can drive its decisions without killing a process, and
    /// its body reads `send(pid)`. Joined by name with the crate's free `fn send`, stopping a
    /// session read as commissioning one.
    ///
    /// Sound for the same reason as the rule above: a parameter is in scope for the whole body and
    /// shadows a function of that name in the value namespace, so `send(pid)` CANNOT be the free
    /// function. It is deliberately parameters only and not `let` bindings — a `let` shadows only
    /// from its own statement onward, so a call above it would still be the function, and dropping
    /// that edge would err toward UNREACHABLE, which is the direction this graph must never take.
    #[test]
    fn a_call_to_a_parameter_is_not_a_call_to_the_function_it_shadows() {
        let src = "fn seam(pid: u32, send: impl Fn(u32)) { send(pid); }\n\
                   fn send(req: R) { funnel(req); }\n";
        let graph = call_graph(src).unwrap();
        let seam = graph.iter().find(|f| f.name == "seam").expect("the node");
        assert!(
            !seam.calls.contains("send"),
            "a call to the parameter was recorded as a call to the function: {seam:?}"
        );
        let reaching = callers_reaching(&graph, &["funnel"]);
        assert_eq!(
            reaching.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["send"],
            "the injected signaller was read as the verb it is named after: {reaching:?}"
        );

        // And a `let` binding of the same name is NOT dropped, because the scope rule does not
        // hold for it: the edge stays and the answer errs toward reachable.
        let shadowed_late = "fn seam(req: R) { send(req); let send = 1; }\n\
                             fn send(req: R) { funnel(req); }\n";
        assert!(
            callers_reaching(&call_graph(shadowed_late).unwrap(), &["funnel"]).contains("seam"),
            "a call made before a `let` of the same name is the function, and has to stay an edge"
        );
    }

    /// `public_fns_lending_mut` has to see the referent wherever it sits — including nested inside
    /// an `impl FnOnce(&mut T) -> …` bound, which is the shape the foundation handle uses.
    #[test]
    fn lending_mut_sees_a_referent_nested_in_a_closure_bound() {
        let src = "impl H {\n    \
                   pub fn with_state<T>(&self, f: impl FnOnce(&State<S>) -> T) -> T { f() }\n    \
                   pub fn with_state_mut<T>(&self, f: impl FnOnce(&mut State<S>) -> T) -> T { f() }\n    \
                   pub fn take(&self, s: &mut State<S>) {}\n    \
                   fn private_mut(&self, s: &mut State<S>) {}\n    \
                   pub fn unrelated(&self, s: &mut Other) {}\n}\n";
        let found = public_fns_lending_mut(src, "State").unwrap();
        assert_eq!(
            found.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["take", "with_state_mut"],
            "{found:?}"
        );
    }

    fn seam(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn verbs(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn seam_name_joins_path_and_kebab_with_underscores() {
        assert_eq!(seam_name("prime"), "prime");
        assert_eq!(seam_name("dep add"), "dep_add");
        assert_eq!(seam_name("workflow step done"), "workflow_step_done");
        assert_eq!(seam_name("agent-manifest"), "agent_manifest");
    }

    #[test]
    fn a_verb_present_on_the_seam_needs_no_waiver() {
        assert_eq!(
            verdict("nxf", &verbs(&["dep add"]), &seam(&["dep_add"]), &[]),
            None
        );
    }

    #[test]
    fn a_verb_missing_from_the_seam_fails_and_names_what_it_looked_for() {
        let complaint = verdict("nxm", &verbs(&["import"]), &seam(&["recall"]), &[])
            .expect("an uncovered verb must fail the gate");
        assert!(complaint.contains("nxm import"), "{complaint}");
        assert!(complaint.contains("`import`"), "{complaint}");
    }

    /// The event this gate was built for, in miniature.
    ///
    /// At v0.41.0 (commit 619d8c76, the last release before the two lifts) the `nxm` seam carried
    /// `recall`/`memories`/`remember`/`forget` and no `prime` at all — the session-start assembly
    /// was a private function in `cli.rs` — while all three parity differentials were green and
    /// app-foundations was rebuilding that same assembly in TypeScript. The seam list below is the
    /// real one from that commit; run against it, the gate names the verb nobody could see.
    #[test]
    fn the_omission_this_gate_was_built_for_would_have_been_caught() {
        let pre_lift = seam(&[
            "open",
            "open_with_poll_interval",
            "subscribe",
            "recall",
            "memories",
            "remember",
            "forget",
            "workspace",
        ]);
        let complaint = verdict("nxm", &verbs(&["prime", "recall"]), &pre_lift, &[])
            .expect("a CLI-only session start must fail the gate");
        assert!(complaint.contains("nxm prime"), "{complaint}");
    }

    #[test]
    fn each_waiver_kind_covers_its_verb() {
        let cases = [
            Waiver::CliOnly {
                verb: "init",
                reason: "sets up the host, not the workspace",
            },
            Waiver::Alias {
                verb: "init",
                seam: "recall",
                reason: "named differently",
            },
            Waiver::KnownGap {
                verb: "init",
                ticket: "6j6v.1234",
                reason: "not lifted yet",
            },
        ];
        for waiver in cases {
            assert_eq!(
                verdict("nxm", &verbs(&["init"]), &seam(&["recall"]), &[waiver]),
                None,
                "{waiver:?} should cover its verb"
            );
        }
    }

    #[test]
    fn a_waiver_without_a_reason_is_itself_a_finding() {
        let complaint = verdict(
            "nxm",
            &verbs(&["import"]),
            &seam(&["recall"]),
            &[Waiver::CliOnly {
                verb: "import",
                reason: "   ",
            }],
        )
        .expect("a reasonless waiver must fail the gate");
        assert!(complaint.contains("carries no reason"), "{complaint}");
    }

    #[test]
    fn a_known_gap_without_a_ticket_is_not_a_waiver() {
        let complaint = verdict(
            "nxc",
            &verbs(&["transcript append"]),
            &seam(&["inbox"]),
            &[Waiver::KnownGap {
                verb: "transcript append",
                ticket: "",
                reason: "no transcript write on the seam",
            }],
        )
        .expect("a ticketless gap must fail the gate");
        assert!(complaint.contains("names no board item"), "{complaint}");
    }

    #[test]
    fn an_alias_pointing_at_nothing_is_void() {
        let complaint = verdict(
            "nxf",
            &verbs(&["label list"]),
            &seam(&["show"]),
            &[Waiver::Alias {
                verb: "label list",
                seam: "labels",
                reason: "the read is named for the collection",
            }],
        )
        .expect("an alias to a missing symbol must fail the gate");
        assert!(complaint.contains("not on the seam"), "{complaint}");
    }

    #[test]
    fn a_waiver_for_a_verb_that_no_longer_exists_must_be_dropped() {
        let complaint = verdict(
            "nxf",
            &verbs(&["show"]),
            &seam(&["show"]),
            &[Waiver::CliOnly {
                verb: "beads-import",
                reason: "migration path",
            }],
        )
        .expect("a stale waiver must fail the gate");
        assert!(complaint.contains("no longer a nxf verb"), "{complaint}");
    }

    #[test]
    fn a_waiver_for_a_verb_the_seam_has_grown_must_be_dropped() {
        let complaint = verdict(
            "nxm",
            &verbs(&["prime"]),
            &seam(&["prime"]),
            &[Waiver::KnownGap {
                verb: "prime",
                ticket: "6j6v.wph0",
                reason: "assembled in the CLI",
            }],
        )
        .expect("a paid-off gap must fail the gate");
        assert!(complaint.contains("the debt is paid"), "{complaint}");
    }

    #[test]
    fn one_verb_may_carry_only_one_reason() {
        let complaint = verdict(
            "nxc",
            &verbs(&["session bind"]),
            &seam(&["inbox"]),
            &[
                Waiver::CliOnly {
                    verb: "session bind",
                    reason: "sidecar callback",
                },
                Waiver::KnownGap {
                    verb: "session bind",
                    ticket: "6j6v.1234",
                    reason: "also a gap?",
                },
            ],
        )
        .expect("a doubly-waived verb must fail the gate");
        assert!(complaint.contains("listed 2 times"), "{complaint}");
    }

    #[test]
    fn public_symbols_finds_free_functions_and_inherent_methods() {
        let src = r#"
            pub fn recall(key: &str) {}
            fn private_helper() {}
            pub struct Engine;
            impl Engine {
                pub fn prime(&self) {}
                fn hidden(&self) {}
            }
            impl std::fmt::Debug for Engine {
                fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
            }
            pub mod nested {
                pub fn deep(&self) {}
            }
            mod private_mod {
                pub fn unreachable() {}
            }
        "#;
        let found = public_symbols(src).expect("parses");
        assert_eq!(
            found,
            seam(&["recall", "prime", "deep"]),
            "only publicly reachable inherent functions are seam"
        );
    }

    #[test]
    fn leaf_verbs_walks_leaves_keeps_hidden_and_drops_clap_help() {
        let cli = clap::Command::new("nxc")
            .subcommand(clap::Command::new("send"))
            .subcommand(clap::Command::new("prime").hide(true))
            .subcommand(clap::Command::new("help"))
            .subcommand(
                clap::Command::new("workflow")
                    .subcommand(clap::Command::new("step").subcommand(clap::Command::new("done"))),
            );
        assert_eq!(
            leaf_verbs(&cli),
            verbs(&["prime", "send", "workflow step done"])
        );
    }

    /// A lift can land as a re-export rather than a definition, and the gate has to see it — else it
    /// reports a false omission and the fix is a waiver written to silence it.
    #[test]
    fn public_symbols_follows_pub_use_re_exports() {
        let src = r#"
            pub use inner::channel_open;
            pub use inner::{join, leave};
            pub use inner::create as channel_create;
            pub use inner::*;
            use inner::private_import;
        "#;
        let found = public_symbols(src).expect("parses");
        assert_eq!(
            found,
            seam(&["channel_open", "join", "leave", "channel_create"]),
            "leaf names and `as` renames are seam; a private `use` is not"
        );
        assert!(
            !found.contains("private_import"),
            "a non-pub `use` re-exports nothing"
        );
    }

    /// The gate's OWN entry point, not just the pure helper underneath it: a regression that logged
    /// the complaint instead of panicking would leave every module's gate green forever.
    #[test]
    #[should_panic(expected = "VERB SEAM PARITY")]
    fn assert_verb_seam_panics_on_an_uncovered_verb() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "pub fn recall() {}\n").unwrap();
        let cli = clap::Command::new("nxm").subcommand(clap::Command::new("summarize"));
        assert_verb_seam("nxm", &cli, &[src], &[]);
    }

    /// The other panic path: an unreadable seam must abort with the BLIND wording, not the
    /// missing-verb wording — the two send a reader to opposite places.
    #[test]
    #[should_panic(expected = "VERB SEAM GATE IS BLIND")]
    fn assert_verb_seam_panics_loudly_when_the_seam_cannot_be_read() {
        let cli = clap::Command::new("nxm").subcommand(clap::Command::new("recall"));
        assert_verb_seam("nxm", &cli, &[PathBuf::from("/nonexistent/engine.rs")], &[]);
    }

    /// A clean seam with every verb answered must NOT panic — the counterpart to the two above, so
    /// "it always panics" cannot pass for "it panics when it should".
    #[test]
    fn assert_verb_seam_stays_silent_when_the_seam_holds() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "pub fn recall() {}\npub fn dep_add() {}\n").unwrap();
        let cli = clap::Command::new("nxf")
            .subcommand(clap::Command::new("recall"))
            .subcommand(clap::Command::new("dep").subcommand(clap::Command::new("add")));
        assert_verb_seam("nxf", &cli, &[src], &[]);
    }

    /// An absolute source path would silently resolve away from the crate, and the resulting
    /// "missing file" would masquerade as a moved crate. Fail on the real cause instead.
    #[test]
    #[should_panic(expected = "ABSOLUTE path")]
    fn crate_relative_refuses_an_absolute_source() {
        crate_relative("/repo/crates/memory", "/etc/passwd");
    }

    #[test]
    fn crate_relative_resolves_relative_sources_from_the_manifest_dir() {
        assert_eq!(
            crate_relative("/repo/crates/cli", "../facade/src/read.rs"),
            PathBuf::from("/repo/crates/facade/src/read.rs")
        );
        assert_eq!(
            crate_relative("/repo/crates/memory", "./src/engine.rs"),
            PathBuf::from("/repo/crates/memory/src/engine.rs")
        );
    }

    #[test]
    fn read_seam_reports_a_source_that_is_not_valid_rust() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "pub fn broken( {\n").unwrap();
        let err = read_seam(&[src]).expect_err("unparseable Rust must be an error");
        assert!(err.contains("could not be parsed as Rust"), "{err}");
    }

    #[test]
    fn read_seam_refuses_to_report_all_clear_when_it_cannot_see() {
        let missing = PathBuf::from("/nonexistent/engine.rs");
        let err = read_seam(&[missing]).expect_err("an unreadable seam source must be an error");
        assert!(err.contains("could not be read"), "{err}");
        assert!(
            read_seam(&[]).is_err(),
            "an empty seam source list must be an error"
        );
    }

    #[test]
    fn read_seam_rejects_a_source_with_no_public_surface() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "fn private_only() {}\n").unwrap();
        let err = read_seam(&[src]).expect_err("a source with no public fn must be an error");
        assert!(err.contains("no public functions"), "{err}");
    }
}
