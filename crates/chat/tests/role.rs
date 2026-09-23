use nexus_chat::role::*;

#[test]
fn loads_with_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("coding.yaml");
    std::fs::write(
        &p,
        "handle: coding\nsystem_prompt: |\n  Do it.\ntools: [Read, Bash]\n",
    )
    .unwrap();
    let r = load_role(&p).unwrap();
    assert_eq!(r.handle, "coding");
    assert_eq!(r.tools, Some(vec!["Read".to_string(), "Bash".to_string()]));
    assert!(matches!(r.base_prompt, BasePrompt::ClaudeCode));
    assert!(matches!(r.session, SessionPolicy::Fresh));
}

#[test]
fn omitted_tools_round_trips_to_none() {
    // Fix round (6j6v.zenf final review, Fix 1): a role YAML that omits `tools:` entirely must
    // deserialize to `None`, NOT an empty `Vec` — the sidecar distinguishes "no intent declared"
    // (leave the SDK's default toolset alone) from "explicitly zero tools" (disable it), and a
    // `Vec<String>` could never represent the first case since it always serializes, even empty.
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("no-tools.yaml");
    std::fs::write(&p, "handle: c\nsystem_prompt: ROLE\n").unwrap();
    let r = load_role(&p).unwrap();
    assert_eq!(r.tools, None, "omitted `tools:` must round-trip to None");
}

#[test]
fn explicit_empty_tools_round_trips_to_some_empty() {
    // The companion case: an author who explicitly writes `tools: []` DOES mean "zero tools" —
    // that must stay distinguishable from the omitted case above.
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("empty-tools.yaml");
    std::fs::write(&p, "handle: c\nsystem_prompt: ROLE\ntools: []\n").unwrap();
    let r = load_role(&p).unwrap();
    assert_eq!(
        r.tools,
        Some(vec![]),
        "explicit `tools: []` must stay Some(vec![])"
    );
}

#[test]
fn compose_appends_preset_and_inherits_claude_md() {
    // prime:false — this test is about base_prompt/claude_md composition, not priming (6j6v.cvsp
    // adds a prime block ahead of everything else when `prime` is true, which defaults to true and
    // would otherwise leak into this exact-match assertion).
    let r: RoleDecl = serde_yaml::from_str(
        "handle: c\nsystem_prompt: ROLE\nbase_prompt: claude_code\nclaude_md: inherit\nprime: false\n",
    )
    .unwrap();
    let c = compose_system_prompt(&r, Some("PROJECT"), None, "", None);
    assert!(c.use_claude_code_preset);
    assert_eq!(c.system_prompt, "PROJECT\n\nROLE");
}

#[test]
fn compose_none_stands_alone_and_ignore_drops_claude_md() {
    // prime:false — see note above.
    let r: RoleDecl = serde_yaml::from_str(
        "handle: c\nsystem_prompt: ROLE\nbase_prompt: none\nclaude_md: ignore\nprime: false\n",
    )
    .unwrap();
    let c = compose_system_prompt(&r, Some("PROJECT"), None, "", None);
    assert!(!c.use_claude_code_preset);
    assert_eq!(c.system_prompt, "ROLE");
}

#[test]
fn compose_override_drops_claude_md_exactly_as_ignore_does() {
    // The guide's "Declared but not yet read" list calls `claude_md: override` unread, and that is
    // only HALF true (nxf 6j6v.mg5b): the per-persona replacement document is unspecified, so
    // nothing composes one — but `override` is not `inherit`, so declaring it silently composes the
    // persona WITHOUT the project's `CLAUDE.md`. An author who reads "not read yet" as "no effect"
    // loses the project's conventions from that persona's prompt. The guide now says so, and this
    // is what holds the sentence: `override` must compose byte-identically to `ignore`.
    let yaml = |policy| {
        format!("handle: c\nsystem_prompt: ROLE\nbase_prompt: none\nclaude_md: {policy}\nprime: false\n")
    };
    let over: RoleDecl = serde_yaml::from_str(&yaml("override")).unwrap();
    let ignore: RoleDecl = serde_yaml::from_str(&yaml("ignore")).unwrap();
    let composed = compose_system_prompt(&over, Some("PROJECT"), None, "", None);
    assert_eq!(
        composed.system_prompt,
        compose_system_prompt(&ignore, Some("PROJECT"), None, "", None).system_prompt,
        "`override` composes exactly what `ignore` does today"
    );
    assert_eq!(
        composed.system_prompt, "ROLE",
        "and that means the project's CLAUDE.md is NOT in the composed prompt"
    );
}

#[test]
fn prime_defaults_to_true_when_yaml_omits_it() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("no-prime.yaml");
    std::fs::write(&p, "handle: c\nsystem_prompt: ROLE\n").unwrap();
    let r = load_role(&p).unwrap();
    assert!(
        r.prime.is_primed(),
        "omitted `prime:` must default to every service"
    );
}

/// **What a primed role's prompt is made of, after nxf 6j6v.k8zq.** This test used to pin
/// `nxc_usage_block`'s four hand-written verbs — the third of three copies of the same `nxc`
/// guidance, and the only one a spawned persona ever read. What it pins now is the rule that
/// replaced it: the composed block is passed IN, whole, and the identity is not restated beside it.
#[test]
fn prime_true_carries_the_composed_block_and_the_role_prompt() {
    let r: RoleDecl = serde_yaml::from_str(
        "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: true\njob_title: Coder\njob_description: Implements features and fixes bugs.\n",
    )
    .unwrap();
    // Stands in for what `nxs prime --persona c` composes: chat's own block carries the identity,
    // the command reference and the answering rules, and it is handed over verbatim.
    let block = "# nexus-chat\n\n## You are `c`\n- **Role:** Coder\n\n                 - `nxc send --to <persona|channel> --ref nxf_ids=<id> \"<body>\"` — start a                  conversation";
    let c = compose_system_prompt(&r, None, None, block, None);
    assert!(
        c.system_prompt.starts_with(block),
        "the composed block leads, whole and unedited, got:\n{}",
        c.system_prompt
    );
    assert!(
        c.system_prompt.ends_with("ROLE"),
        "the role's own system_prompt must still be present, got:\n{}",
        c.system_prompt
    );
    assert!(
        !c.system_prompt.contains("Job title: Coder"),
        "the identity is NOT restated beside the block that already carries it — two copies of one \
fact in one prompt is where they start to disagree, got:\n{}",
        c.system_prompt
    );
}

#[test]
fn prime_false_omits_the_composed_block_and_the_job_description() {
    let r: RoleDecl = serde_yaml::from_str(
        "handle: c\nsystem_prompt: ROLE\nclaude_md: ignore\nprime: false\njob_title: Coder\njob_description: Implements features and fixes bugs.\n",
    )
    .unwrap();
    let c = compose_system_prompt(&r, None, None, "# nexus-chat\n\n## You are `c`", None);
    assert_eq!(
        c.system_prompt, "ROLE",
        "prime:false must compose exactly the role's own system_prompt, nothing prepended"
    );
    assert!(!c.system_prompt.contains("nexus-chat"));
    assert!(!c
        .system_prompt
        .contains("Implements features and fixes bugs."));
}
