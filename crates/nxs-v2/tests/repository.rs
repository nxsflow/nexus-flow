//! `nxs repository list` (RFC 0009, "Public surfaces this RFC brings"): the repository records of
//! the coordination space, in name order, each with `name`, `space` and `left`; and the parity of
//! every coordination command with its `Engine` counterpart: the same data gives the same result.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, json, nxs, options, scratch, stdout};
use nexus_flow_seam::Engine;

fn repo(base: &Path, folder: &str) -> PathBuf {
    let repo = base.join(folder);
    fs::create_dir_all(&repo).unwrap();
    let out = nxs(base, &repo, &["init"]);
    assert_eq!(out.code, 0, "{}", stdout(&out));
    repo
}

fn space_of(repo: &Path) -> String {
    let text = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    let line = text.lines().find(|l| l.starts_with("repository")).unwrap();
    line.split('"').nth(1).unwrap().to_owned()
}

fn golden(name: &str, one: &str, two: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(path)
        .unwrap()
        .replace("<first>", one)
        .replace("<second>", two)
}

fn engine(base: &Path, repo: &Path) -> Engine {
    block_on(Engine::open_with(repo, options(&base.join("data")))).unwrap()
}

#[test]
fn the_list_prints_every_record_in_name_order_as_the_golden_files() {
    let base = scratch("repository-list");
    let first = repo(&base, "first");
    let second = repo(&base, "second");
    let created = nxs(
        &base,
        &first,
        &["space", "create-coordination", "--name", "central-infra"],
    );
    let coordination = json(&created)["coordination"].as_str().unwrap().to_owned();
    nxs(
        &base,
        &second,
        &["space", "join-coordination", &coordination, "--name", "app"],
    );

    let listed = nxs(&base, &first, &["repository", "list"]);
    let listed_md = nxs(&base, &first, &["repository", "list", "--md"]);

    let (one, two) = (space_of(&first), space_of(&second));
    assert_eq!(listed.code, 0, "{}", stdout(&listed));
    assert_eq!(stdout(&listed), golden("repository-list.json", &one, &two));
    assert_eq!(stdout(&listed_md), golden("repository-list.md", &one, &two));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn listing_in_a_repository_of_no_coordination_space_is_validation() {
    let base = scratch("repository-list-none");
    let repo = repo(&base, "repo");

    let out = nxs(&base, &repo, &["repository", "list"]);

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "validation");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn every_command_gives_what_its_engine_counterpart_gives() {
    let cli = scratch("parity-cli");
    let lib = scratch("parity-lib");
    let (cli_first, cli_second) = (repo(&cli, "first"), repo(&cli, "second"));
    let (lib_first, lib_second) = (repo(&lib, "first"), repo(&lib, "second"));

    let created = json(&nxs(
        &cli,
        &cli_first,
        &["space", "create-coordination", "--name", "a"],
    ));
    let cli_space = created["coordination"].as_str().unwrap().to_owned();
    let joined = json(&nxs(
        &cli,
        &cli_second,
        &["space", "join-coordination", &cli_space, "--name", "b"],
    ));
    let listed = json(&nxs(&cli, &cli_first, &["repository", "list"]));
    let left = json(&nxs(&cli, &cli_second, &["space", "leave-coordination"]));

    let lib_created = block_on(engine(&lib, &lib_first).coordination_create(Some("a"))).unwrap();
    let lib_joined =
        block_on(engine(&lib, &lib_second).coordination_join(lib_created.coordination, Some("b")))
            .unwrap();
    let lib_listed = block_on(engine(&lib, &lib_first).repositories()).unwrap();
    let lib_left = block_on(engine(&lib, &lib_second).coordination_leave()).unwrap();

    // The spaces differ between the two runs; everything else is the same.
    let shape = |value: &serde_json::Value, coordination: &str, first: &str, second: &str| {
        value
            .to_string()
            .replace(coordination, "<coordination>")
            .replace(first, "<first>")
            .replace(second, "<second>")
    };
    let cli_ids = (
        cli_space.as_str(),
        space_of(&cli_first),
        space_of(&cli_second),
    );
    let lib_space = lib_created.coordination.to_string();
    let lib_ids = (
        lib_space.as_str(),
        space_of(&lib_first),
        space_of(&lib_second),
    );
    for (printed, coordination) in [
        (created, Some(&lib_created)),
        (joined, Some(&lib_joined)),
        (left, Some(&lib_left)),
        (listed, None),
    ] {
        let from_lib = match coordination {
            Some(c) => serde_json::json!({
                "coordination": c.coordination.to_string(),
                "written": c.written,
                "repository": c.repository.as_ref().map(|r| serde_json::json!({
                    "name": r.name, "space": r.space, "left": r.left,
                })),
            }),
            None => serde_json::json!(lib_listed
                .iter()
                .map(|r| serde_json::json!({"name": r.name, "space": r.space, "left": r.left}))
                .collect::<Vec<_>>()),
        };
        assert_eq!(
            shape(&printed, cli_ids.0, &cli_ids.1, &cli_ids.2),
            shape(&from_lib, lib_ids.0, &lib_ids.1, &lib_ids.2)
        );
    }
    fs::remove_dir_all(&cli).unwrap();
    fs::remove_dir_all(&lib).unwrap();
}
