use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use mission_workspace::{BaselineSelection, GitRunner, RouteWorkspaceManager, WorkspaceError};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(root: &Path) {
    fs::create_dir_all(root).expect("repo");
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(root.join("tracked.txt"), b"baseline\n").expect("write");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-qm", "baseline"]);
}

#[test]
fn worktree_is_created_on_an_opaque_route_branch_without_touching_user_changes() {
    let temp = tempfile::tempdir().expect("temp");
    let source = temp.path().join("source");
    let managed = temp.path().join("managed");
    repo(&source);
    fs::write(source.join("tracked.txt"), b"user change\n").expect("dirty");
    fs::write(source.join("untracked.txt"), b"keep me\n").expect("untracked");
    let before = git(&source, &["status", "--porcelain=v1", "--untracked-files=all"]);
    let head = git(&source, &["rev-parse", "HEAD"]);
    let manager = RouteWorkspaceManager::new(
        GitRunner::new(OsStr::new("git"), &source, Duration::from_secs(10)).expect("runner"),
        &managed,
    )
    .expect("manager");

    let route = manager
        .create("mission-0123456789", "route-abcdef0123", BaselineSelection::CommittedHead(head.clone()))
        .expect("create route worktree");

    assert_eq!(route.branch, "mission/mission0/routeabc");
    assert!(route.path.starts_with(managed.canonicalize().expect("canonical managed root")));
    assert_eq!(git(&route.path, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&source, &["status", "--porcelain=v1", "--untracked-files=all"]), before);
}

#[test]
fn worktree_rejects_existing_branch_or_nonempty_target() {
    let temp = tempfile::tempdir().expect("temp");
    let source = temp.path().join("source");
    let managed = temp.path().join("managed");
    repo(&source);
    let head = git(&source, &["rev-parse", "HEAD"]);
    let manager = RouteWorkspaceManager::new(
        GitRunner::new(OsStr::new("git"), &source, Duration::from_secs(10)).expect("runner"),
        &managed,
    )
    .expect("manager");
    manager.create("mission-0123456789", "route-abcdef0123", BaselineSelection::CommittedHead(head.clone())).expect("first");

    assert!(matches!(manager.create("mission-0123456789", "route-abcdef0123", BaselineSelection::CommittedHead(head)), Err(WorkspaceError::BranchExists(_)) | Err(WorkspaceError::TargetNotEmpty(_))));
}
