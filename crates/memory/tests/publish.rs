//! `nxm doc --publish` against a REAL temporary repository with a file-path origin (6j6v.g0ye).
//!
//! The git half is real, every call of it. The forge is the one thing replaced, by a recording
//! double behind [`Forge`], because no test may reach GitHub; the production `gh` path is driven in
//! `tests/project_doc.rs` with a stand-in `gh` on `PATH`.

use nexus_memory::error::{NxfError, Result};
use nexus_memory::publish::{self, Forge, Outcome, BRANCH};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const DOC: &str = "NEXUS_MEMORY.md";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        // `status` must not refresh the index it is about to be compared by.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// A bare file-path origin with one commit on `main`, carrying `NEXUS_MEMORY.md` at `published`,
/// and two clones of it.
struct Repo {
    _tmp: TempDir,
    origin: PathBuf,
    clone: PathBuf,
    second: PathBuf,
}

const PUBLISHED: &str = "# what is on the default branch\n";

fn repo() -> Repo {
    let tmp = TempDir::new().unwrap();
    let origin = tmp.path().join("origin.git");
    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&seed).unwrap();
    git(&origin, &["init", "--bare", "-q", "-b", "main"]);
    git(&seed, &["init", "-q", "-b", "main"]);
    std::fs::write(seed.join(DOC), PUBLISHED).unwrap();
    std::fs::write(seed.join("code.txt"), "code\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "seed"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "-q", "origin", "main"]);
    let clone = tmp.path().join("clone");
    let second = tmp.path().join("second");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            second.to_str().unwrap(),
        ],
    );
    Repo {
        _tmp: tmp,
        origin,
        clone,
        second,
    }
}

/// Advance `main` on the origin from a scratch clone, as a merge elsewhere would.
fn advance_main(repo: &Repo, file: &str, content: &str) -> String {
    let scratch = repo
        .origin
        .parent()
        .unwrap()
        .join(format!("scratch-{file}"));
    git(
        repo.origin.parent().unwrap(),
        &[
            "clone",
            "-q",
            repo.origin.to_str().unwrap(),
            scratch.to_str().unwrap(),
        ],
    );
    std::fs::write(scratch.join(file), content).unwrap();
    git(&scratch, &["add", "."]);
    git(&scratch, &["commit", "-q", "-m", "advance"]);
    git(&scratch, &["push", "-q", "origin", "main"]);
    git(&scratch, &["rev-parse", "HEAD"])
}

/// A recording forge: one open pull request at most, and it can be told to fail.
#[derive(Default)]
struct FakeForge {
    open: RefCell<Option<String>>,
    created: RefCell<Vec<(String, String)>>,
    closed: RefCell<Vec<String>>,
    fail_create: RefCell<Option<String>>,
    fail_lookup: RefCell<Option<String>>,
}

impl Forge for FakeForge {
    fn open_pull_request(&self, _root: &Path, _branch: &str) -> Result<Option<String>> {
        if let Some(msg) = self.fail_lookup.borrow().as_ref() {
            return Err(NxfError::io(msg.clone()));
        }
        Ok(self.open.borrow().clone())
    }
    fn create_pull_request(&self, _root: &Path, branch: &str, base: &str) -> Result<String> {
        if let Some(msg) = self.fail_create.borrow().as_ref() {
            return Err(NxfError::io(msg.clone()));
        }
        self.created
            .borrow_mut()
            .push((branch.to_string(), base.to_string()));
        let url = format!("https://forge.example/pr/{}", self.created.borrow().len());
        *self.open.borrow_mut() = Some(url.clone());
        Ok(url)
    }
    fn close_pull_request(&self, _root: &Path, pull_request: &str) -> Result<()> {
        self.closed.borrow_mut().push(pull_request.to_string());
        *self.open.borrow_mut() = None;
        Ok(())
    }
}

/// Everything of a checkout a publish must leave alone, as bytes.
fn checkout_state(dir: &Path) -> Vec<(String, String)> {
    let read = |p: PathBuf| {
        std::fs::read(&p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_else(|_| "<absent>".into())
    };
    let mut state = vec![
        ("HEAD".to_string(), read(dir.join(".git/HEAD"))),
        ("index".to_string(), read(dir.join(".git/index"))),
        ("FETCH_HEAD".to_string(), read(dir.join(".git/FETCH_HEAD"))),
        ("doc".to_string(), read(dir.join(DOC))),
        ("code".to_string(), read(dir.join("code.txt"))),
        (
            "status".to_string(),
            git(dir, &["status", "--porcelain", "--untracked-files=all"]),
        ),
        (
            "refs".to_string(),
            git(
                dir,
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/heads",
                    "refs/remotes",
                ],
            ),
        ),
    ];
    state.sort();
    state
}

fn origin_ref(repo: &Repo, name: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--verify", "-q", name])
        .current_dir(&repo.origin)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn branch_ref() -> String {
    format!("refs/heads/{BRANCH}")
}

#[test]
fn a_publish_makes_one_commit_on_the_origins_default_and_a_pull_request_touching_no_checkout() {
    let repo = repo();
    // A checkout mid-work: an uncommitted change, a dirty generated file and a staged one.
    std::fs::write(repo.clone.join("code.txt"), "work in progress\n").unwrap();
    std::fs::write(repo.clone.join(DOC), PUBLISHED).unwrap();
    std::fs::write(repo.second.join("staged.txt"), "staged\n").unwrap();
    git(&repo.second, &["add", "staged.txt"]);
    let before = (checkout_state(&repo.clone), checkout_state(&repo.second));
    let main_before = origin_ref(&repo, "refs/heads/main").unwrap();

    let forge = FakeForge::default();
    let outcome = publish::publish("# the store now says this\n", &repo.clone, &forge).unwrap();

    let tip = origin_ref(&repo, &branch_ref()).expect("the branch reached the origin");
    let Outcome::Opened {
        commit,
        pull_request,
    } = outcome
    else {
        panic!("expected a new pull request, got {outcome:?}");
    };
    assert_eq!(commit, tip);
    assert_eq!(pull_request, "https://forge.example/pr/1");
    assert_eq!(
        *forge.created.borrow(),
        vec![(BRANCH.to_string(), "main".to_string())]
    );
    assert_eq!(
        git(&repo.origin, &["rev-parse", &format!("{tip}^")]),
        main_before,
        "the commit sits directly on the origin's default"
    );
    assert_eq!(
        git(&repo.origin, &["show", &format!("{tip}:{DOC}")]),
        "# the store now says this"
    );
    assert_eq!(
        git(
            &repo.origin,
            &["diff-tree", "--no-commit-id", "--name-only", "-r", &tip]
        ),
        DOC,
        "and it changes that one file"
    );
    assert_eq!(
        origin_ref(&repo, "refs/heads/main").unwrap(),
        main_before,
        "the default branch itself moved nowhere"
    );
    assert_eq!(
        before,
        (checkout_state(&repo.clone), checkout_state(&repo.second)),
        "HEAD, index, files and refs of both checkouts come out byte-identical"
    );
}

#[test]
fn a_checkout_behind_the_origin_still_publishes_on_the_origins_tip() {
    let repo = repo();
    let tip_of_main = advance_main(&repo, "later.txt", "merged elsewhere\n");
    let before = checkout_state(&repo.clone);

    publish::publish("# new\n", &repo.clone, &FakeForge::default()).unwrap();

    let tip = origin_ref(&repo, &branch_ref()).unwrap();
    assert_eq!(
        git(&repo.origin, &["rev-parse", &format!("{tip}^")]),
        tip_of_main
    );
    assert_eq!(before, checkout_state(&repo.clone));
}

#[test]
fn a_store_that_says_what_the_default_says_creates_no_branch_and_no_pull_request() {
    let repo = repo();
    let forge = FakeForge::default();
    let outcome = publish::publish(PUBLISHED, &repo.clone, &forge).unwrap();
    assert_eq!(outcome, Outcome::Unchanged { pull_request: None });
    assert_eq!(origin_ref(&repo, &branch_ref()), None);
    assert!(forge.created.borrow().is_empty());
}

#[test]
fn publishing_twice_with_no_change_creates_nothing() {
    let repo = repo();
    let forge = FakeForge::default();
    publish::publish("# new\n", &repo.clone, &forge).unwrap();
    let tip = origin_ref(&repo, &branch_ref()).unwrap();

    let again = publish::publish("# new\n", &repo.clone, &forge).unwrap();

    assert_eq!(
        again,
        Outcome::Unchanged {
            pull_request: Some("https://forge.example/pr/1".into())
        }
    );
    assert_eq!(
        origin_ref(&repo, &branch_ref()).unwrap(),
        tip,
        "no new commit"
    );
    assert_eq!(forge.created.borrow().len(), 1, "no second pull request");
}

#[test]
fn a_publish_with_an_open_pull_request_updates_it_on_the_latest_default() {
    let repo = repo();
    let forge = FakeForge::default();
    publish::publish("# one\n", &repo.clone, &forge).unwrap();
    let tip_of_main = advance_main(&repo, "later.txt", "merged elsewhere\n");

    let outcome = publish::publish("# two\n", &repo.clone, &forge).unwrap();

    let tip = origin_ref(&repo, &branch_ref()).unwrap();
    assert_eq!(
        outcome,
        Outcome::Updated {
            commit: tip.clone(),
            pull_request: "https://forge.example/pr/1".into()
        }
    );
    assert_eq!(forge.created.borrow().len(), 1, "updated, not duplicated");
    assert_eq!(
        git(&repo.origin, &["rev-parse", &format!("{tip}^")]),
        tip_of_main
    );
    assert_eq!(
        git(&repo.origin, &["show", &format!("{tip}:{DOC}")]),
        "# two"
    );
}

#[test]
fn an_open_pull_request_is_closed_when_the_store_comes_back_to_what_the_default_says() {
    // Merging it would then take the file BACKWARDS.
    let repo = repo();
    let forge = FakeForge::default();
    publish::publish("# one\n", &repo.clone, &forge).unwrap();

    let outcome = publish::publish(PUBLISHED, &repo.clone, &forge).unwrap();

    assert_eq!(outcome, Outcome::Unchanged { pull_request: None });
    assert_eq!(
        *forge.closed.borrow(),
        vec!["https://forge.example/pr/1".to_string()]
    );
}

#[test]
fn a_forge_that_will_not_create_fails_the_publish_and_the_next_attempt_finishes_it() {
    let repo = repo();
    let forge = FakeForge::default();
    *forge.fail_create.borrow_mut() = Some("HTTP 401: bad credentials".into());
    let err = publish::publish("# new\n", &repo.clone, &forge).unwrap_err();
    assert!(err.msg.contains("HTTP 401"), "{}", err.msg);

    *forge.fail_create.borrow_mut() = None;
    let outcome = publish::publish("# new\n", &repo.clone, &forge).unwrap();
    assert!(matches!(outcome, Outcome::Opened { .. }), "{outcome:?}");
}

#[test]
fn a_forge_that_cannot_be_asked_leaves_no_branch_behind() {
    // What a workspace with no `gh` meets: nothing reaches the origin, and the reason comes back.
    let repo = repo();
    let forge = FakeForge::default();
    *forge.fail_lookup.borrow_mut() = Some("gh was not found".into());
    let err = publish::publish("# new\n", &repo.clone, &forge).unwrap_err();
    assert!(err.msg.contains("gh was not found"), "{}", err.msg);
    assert_eq!(origin_ref(&repo, &branch_ref()), None);
}

#[test]
fn a_workspace_with_no_origin_is_not_published() {
    let tmp = TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    let forge = FakeForge::default();
    assert_eq!(
        publish::publish("# new\n", tmp.path(), &forge).unwrap(),
        Outcome::NotPublished
    );
    assert!(forge.created.borrow().is_empty());
}

#[test]
fn a_workspace_in_a_subdirectory_publishes_the_file_at_its_place_in_the_repository() {
    let repo = repo();
    let sub = repo.clone.join("packages/app");
    std::fs::create_dir_all(&sub).unwrap();
    publish::publish("# below\n", &sub, &FakeForge::default()).unwrap();
    let tip = origin_ref(&repo, &branch_ref()).unwrap();
    assert_eq!(
        git(
            &repo.origin,
            &["show", &format!("{tip}:packages/app/{DOC}")]
        ),
        "# below"
    );
    assert_eq!(
        git(&repo.origin, &["show", &format!("{tip}:{DOC}")]),
        PUBLISHED.trim_end(),
        "the root's file stays what it was"
    );
}

#[test]
fn a_repository_that_ignores_the_file_is_not_published() {
    let repo = repo();
    std::fs::write(repo.clone.join(".gitignore"), format!("{DOC}\n")).unwrap();
    git(&repo.clone, &["rm", "-q", "--cached", DOC]);
    let forge = FakeForge::default();
    assert_eq!(
        publish::publish("# new\n", &repo.clone, &forge).unwrap(),
        Outcome::NotPublished
    );
    assert_eq!(origin_ref(&repo, &branch_ref()), None);
}
