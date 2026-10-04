//! **The user-level declaration folder at the seam an embedding app reads** (nxf 6j6v.7k58).
//!
//! An app resolves its catalogue through `Engine`, and the folder it reads must be the one the
//! CLI reads — the personas an app starts run `nxc`, and two answers to "who exists" would split
//! the app from its own sessions. So the engine takes no location: it follows the service home, from
//! `$HOME`. Pinning `$HOME` is process-wide, which is why this is ONE test in a binary of its own.

use nexus_chat::definitions::{DeclarationKind, DeclarationOrigin};
use tempfile::TempDir;

const NOW: &str = "2026-10-04T10:00:00Z";

#[test]
fn the_engine_reads_the_user_level_folder_the_cli_reads_and_says_where_each_entry_came_from() {
    let home = TempDir::new().unwrap();
    // SAFETY of the mutation: this binary has exactly one test, and nothing else runs in it.
    std::env::set_var("HOME", home.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    std::env::remove_var("XDG_DATA_HOME");
    // Whichever instance the ambient environment names, the folder is that instance's — resolved
    // the way production resolves it, not spelled out here.
    let user = nexus_chat::definitions::user_declarations_dir().unwrap();
    assert!(user.starts_with(home.path()), "{}", user.display());
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(
        user.join("pm.yaml"),
        "handle: pm\njob_title: Product manager\nsystem_prompt: The central PM.\n",
    )
    .unwrap();
    std::fs::write(
        user.join("channels.yaml"),
        "- name: planning\n  members: [pm]\n",
    )
    .unwrap();

    let repo = TempDir::new().unwrap();
    nexus_chat::workspace::setup(repo.path(), &nexus_chat::workspace::chat_config()).unwrap();
    let engine = nexus_chat::engine::Engine::open(None, repo.path()).unwrap();

    // `definitions`: the merged catalogue, with its account.
    let defs = engine.definitions().unwrap();
    assert_eq!(defs.role("pm").unwrap().system_prompt, "The central PM.");
    let source = defs.source().unwrap();
    assert_eq!(source.user_path.as_deref(), Some(user.as_path()));
    assert_eq!(
        source.origin_of(DeclarationKind::Channel, "planning"),
        DeclarationOrigin::User
    );

    // `directory`: every entry stamped with its origin, the same record `nxc list --json` prints.
    let directory = engine.directory(None).unwrap();
    let pm = directory
        .personas
        .iter()
        .find(|p| p.handle == "pm")
        .unwrap();
    assert_eq!(pm.origin, DeclarationOrigin::User);
    let planning = directory
        .channels
        .iter()
        .find(|c| c.name == "planning")
        .unwrap();
    assert_eq!(planning.origin, DeclarationOrigin::User);
    let wire = serde_json::to_value(&directory).unwrap();
    assert_eq!(wire["personas"][0]["origin"], "user", "{wire}");

    // `prime_as` for the persona: it is told where its declaration lives, because a path in its
    // instructions relative to "this repository" means the repository it runs in.
    let report = engine.prime_as("local/pm", Some("pm"), NOW).unwrap();
    let brief = report.persona.as_ref().expect("a persona brief");
    assert_eq!(brief.declared_in.as_deref(), Some(user.as_path()));
    let rendered = brief.render_markdown();
    assert!(
        rendered.contains("**Declared in:** the user-level folder"),
        "{rendered}"
    );
    // The composed session start a spawned persona is actually handed carries it.
    let start = engine.persona_prime("pm", NOW).unwrap();
    assert!(start.contains("**Declared in:**"), "{start}");
    // ... and the workspace's own declaration of the same name takes over, reported.
    std::fs::create_dir_all(repo.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        repo.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: The repository's own PM.\n",
    )
    .unwrap();
    let defs = engine.definitions().unwrap();
    assert_eq!(
        defs.role("pm").unwrap().system_prompt,
        "The repository's own PM."
    );
    let report = engine.prime_as("local/pm", Some("pm"), NOW).unwrap();
    assert_eq!(report.persona.as_ref().unwrap().declared_in, None);
    assert_eq!(report.declarations.shadowed.len(), 1);
}
