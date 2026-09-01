use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use mission_workspace::{BaselineState, GitRunner, inspect_baseline};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(root: &Path) {
    fs::create_dir_all(root).expect("create repo");
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(root.join("tracked.txt"), b"baseline\n").expect("write tracked");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-qm", "baseline"]);
}

fn runner(root: &Path) -> GitRunner {
    GitRunner::new(OsStr::new("git"), root, Duration::from_secs(10)).expect("runner")
}

#[test]
fn baseline_clean_and_detached_head_use_committed_head() {
    let temp = tempfile::tempdir().expect("temp");
    repo(temp.path());
    let head = git(temp.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        inspect_baseline(&runner(temp.path())).expect("baseline"),
        BaselineState::Clean { head: head.clone() }
    );

    git(temp.path(), &["checkout", "--detach", "-q", "HEAD"]);
    assert_eq!(
        inspect_baseline(&runner(temp.path())).expect("detached baseline"),
        BaselineState::DetachedHead { head }
    );
}

#[test]
fn baseline_dirty_variants_require_explicit_selection_without_mutation() {
    for variant in ["staged", "unstaged", "untracked"] {
        let temp = tempfile::tempdir().expect("temp");
        repo(temp.path());
        match variant {
            "staged" => {
                fs::write(temp.path().join("tracked.txt"), b"staged\n").expect("write");
                git(temp.path(), &["add", "tracked.txt"]);
            }
            "unstaged" => fs::write(temp.path().join("tracked.txt"), b"unstaged\n").expect("write"),
            "untracked" => fs::write(temp.path().join("new.txt"), b"untracked\n").expect("write"),
            _ => unreachable!(),
        }
        let before = git(
            temp.path(),
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        assert_eq!(
            inspect_baseline(&runner(temp.path())).expect("baseline"),
            BaselineState::SelectionRequired {
                code: "BASELINE_SELECTION_REQUIRED"
            }
        );
        let after = git(
            temp.path(),
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        assert_eq!(after, before, "{variant} workspace changed");
    }
}

#[test]
fn baseline_inspection_supports_a_linked_worktree() {
    let temp = tempfile::tempdir().expect("temp");
    let main = temp.path().join("main");
    let linked = temp.path().join("linked");
    repo(&main);
    git(
        &main,
        &[
            "worktree",
            "add",
            "-qb",
            "linked-test",
            linked.to_str().expect("utf8 path"),
            "HEAD",
        ],
    );
    assert!(matches!(
        inspect_baseline(&runner(&linked)).expect("linked baseline"),
        BaselineState::Clean { .. }
    ));
}
