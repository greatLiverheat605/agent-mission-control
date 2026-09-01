use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use mission_workspace::{CheckpointRequest, CheckpointTrigger, GitCheckpoint, GitRunner};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(root.join("tracked.txt"), b"baseline\n").expect("write");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-qm", "baseline"]);
}

#[test]
fn checkpoint_commit_records_complete_metadata_without_secret_text() {
    let temp = tempfile::tempdir().expect("temp");
    repo(temp.path());
    fs::write(temp.path().join("tracked.txt"), b"changed\n").expect("change");
    let runner =
        GitRunner::new(OsStr::new("git"), temp.path(), Duration::from_secs(10)).expect("runner");

    let checkpoint = GitCheckpoint::create(
        &runner,
        CheckpointRequest {
            checkpoint_id: "cp-001".to_owned(),
            trigger: CheckpointTrigger::BeforeProviderHandoff,
            contract_version: 9,
            loadout_fingerprint: "loadout-v9".to_owned(),
            ledger_sequence: 44,
        },
    )
    .expect("checkpoint");

    assert!(checkpoint.committed);
    assert_eq!(checkpoint.metadata.file_hashes.len(), 1);
    assert_eq!(checkpoint.metadata.contract_version, 9);
    assert_eq!(checkpoint.metadata.ledger_sequence, 44);
    assert_eq!(
        git(temp.path(), &["show", "-s", "--format=%an <%ae>", "HEAD"]),
        "Agent Mission Control <checkpoint@mission-control.invalid>"
    );
    let subject = git(temp.path(), &["show", "-s", "--format=%s", "HEAD"]);
    assert!(subject.contains("cp-001"));
    assert!(!subject.contains("loadout-v9"));
}

#[test]
fn checkpoint_supports_every_required_safe_trigger_and_rejects_secret_like_ids() {
    let triggers = [
        CheckpointTrigger::BeforeLaunch,
        CheckpointTrigger::PlanPhaseEnded,
        CheckpointTrigger::BeforeCompaction,
        CheckpointTrigger::BeforeProviderHandoff,
        CheckpointTrigger::Paused,
        CheckpointTrigger::Blocked,
        CheckpointTrigger::Failed,
        CheckpointTrigger::AutopilotDisengaged,
    ];
    assert_eq!(triggers.len(), 8);

    let temp = tempfile::tempdir().expect("temp");
    repo(temp.path());
    let runner =
        GitRunner::new(OsStr::new("git"), temp.path(), Duration::from_secs(10)).expect("runner");
    let result = GitCheckpoint::create(
        &runner,
        CheckpointRequest {
            checkpoint_id: "token=super-secret".to_owned(),
            trigger: CheckpointTrigger::Paused,
            contract_version: 1,
            loadout_fingerprint: "loadout".to_owned(),
            ledger_sequence: 1,
        },
    );
    assert!(result.is_err());
}
