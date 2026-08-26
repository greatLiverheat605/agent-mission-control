use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use mission_domain::{CriterionEvidence, CriterionStatus, EvidenceMatrix};
use mission_ledger::blob_store::EncryptedBlobStore;
use mission_workspace::{
    ApprovedMerge, DiffPreview, DiffPreviewRequest, GitRunner, MergeActor, MergeApproval,
    MergeError, MergeOutcome, MergeRequest, MergeStrategy,
};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(root: &Path, conflict: bool) {
    git(root, &["init", "-qb", "main"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "core.autocrlf", "false"]);
    fs::write(root.join("tracked.txt"), b"baseline\n").expect("baseline");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-qm", "baseline"]);

    git(root, &["checkout", "-qb", "route"]);
    fs::write(root.join("tracked.txt"), b"route change\n").expect("route change");
    git(root, &["commit", "-qam", "route checkpoint"]);
    git(root, &["checkout", "-q", "main"]);
    if conflict {
        fs::write(root.join("tracked.txt"), b"target change\n").expect("target change");
        git(root, &["commit", "-qam", "target change"]);
    }
}

fn runner(root: &Path) -> GitRunner {
    GitRunner::new(OsStr::new("git"), root, Duration::from_secs(10)).expect("runner")
}

fn preview(root: &Path, store: &EncryptedBlobStore) -> DiffPreview {
    DiffPreview::create(
        &runner(root),
        store,
        DiffPreviewRequest {
            target_branch: "main".to_owned(),
            source_branch: "route".to_owned(),
            validation_commands: vec!["cargo test".to_owned()],
            risks: vec!["changes tracked.txt".to_owned()],
        },
    )
    .expect("preview")
}

fn approval(preview: &DiffPreview, actor: MergeActor) -> MergeApproval {
    MergeApproval::new(
        "approval-merge-1",
        actor,
        preview.digest(),
        preview.target_head.clone(),
    )
    .expect("approval")
}

fn request(preview: &DiffPreview, strategy: MergeStrategy) -> MergeRequest {
    MergeRequest {
        expected_target_head: preview.target_head.clone(),
        strategy,
    }
}

#[test]
fn evidence_matrix_keeps_unverified_criteria_visible_and_blocks_completion() {
    let mut matrix = EvidenceMatrix::from_criteria([
        CriterionEvidence::new("tests", "Automated tests"),
        CriterionEvidence::new("review", "User diff review"),
    ]);
    assert!(matrix.can_await_acceptance());
    assert!(!matrix.is_complete());
    assert!(
        matrix
            .criteria
            .iter()
            .all(|item| item.status == CriterionStatus::Unverified)
    );

    matrix
        .record("tests", CriterionStatus::Verified, ["evidence-test-1"])
        .expect("record test evidence");
    matrix
        .record(
            "review",
            CriterionStatus::NotApplicable,
            std::iter::empty::<&str>(),
        )
        .expect("record not applicable");
    assert!(matrix.is_complete());
}

#[test]
fn preview_contains_target_range_stat_encrypted_diff_and_conflict_precheck() {
    let temp = tempfile::tempdir().expect("temp");
    let repo_path = temp.path().join("repo");
    fs::create_dir(&repo_path).expect("repo dir");
    repo(&repo_path, false);
    let store = EncryptedBlobStore::open(temp.path().join("blobs"), [7; 32]).expect("store");

    let preview = preview(&repo_path, &store);

    assert_eq!(preview.target_branch, "main");
    assert_eq!(preview.source_branch, "route");
    assert_eq!(
        preview.commit_range,
        format!("{}..{}", preview.target_head, preview.source_head)
    );
    assert!(preview.stat.contains("tracked.txt"));
    assert_eq!(preview.validation_commands, ["cargo test"]);
    assert_eq!(preview.risks, ["changes tracked.txt"]);
    assert!(!preview.conflict_precheck.has_conflicts);
    let diff = store
        .read(&preview.diff_blob)
        .expect("read encrypted diff blob");
    assert!(
        String::from_utf8(diff)
            .expect("utf8 diff")
            .contains("route change")
    );
}

#[test]
fn user_merge_preserves_history_or_squashes_and_consumes_approval_once() {
    for (strategy, expected_parents) in [
        (MergeStrategy::PreserveCheckpointHistory, 2),
        (MergeStrategy::Squash, 1),
    ] {
        let temp = tempfile::tempdir().expect("temp");
        let repo_path = temp.path().join("repo");
        fs::create_dir(&repo_path).expect("repo dir");
        repo(&repo_path, false);
        let store = EncryptedBlobStore::open(temp.path().join("blobs"), [9; 32]).expect("store");
        let preview = preview(&repo_path, &store);
        let mut approval = approval(&preview, MergeActor::User);

        let outcome = ApprovedMerge::execute(
            &runner(&repo_path),
            &preview,
            &mut approval,
            request(&preview, strategy),
        )
        .expect("approved merge");
        assert!(matches!(outcome, MergeOutcome::Merged { .. }));
        assert!(approval.is_consumed());
        assert_eq!(
            fs::read_to_string(repo_path.join("tracked.txt")).expect("merged"),
            "route change\n"
        );
        assert_eq!(
            git(&repo_path, &["show", "-s", "--format=%P", "HEAD"])
                .split_whitespace()
                .count(),
            expected_parents
        );
        assert!(matches!(
            ApprovedMerge::execute(
                &runner(&repo_path),
                &preview,
                &mut approval,
                request(&preview, strategy),
            ),
            Err(MergeError::ApprovalConsumed)
        ));
    }
}

#[test]
fn agent_is_denied_and_target_head_drift_requires_a_new_preview() {
    let temp = tempfile::tempdir().expect("temp");
    let repo_path = temp.path().join("repo");
    fs::create_dir(&repo_path).expect("repo dir");
    repo(&repo_path, false);
    let store = EncryptedBlobStore::open(temp.path().join("blobs"), [11; 32]).expect("store");
    let preview = preview(&repo_path, &store);

    let mut agent_approval = approval(&preview, MergeActor::Agent);
    assert!(matches!(
        ApprovedMerge::execute(
            &runner(&repo_path),
            &preview,
            &mut agent_approval,
            request(&preview, MergeStrategy::Squash),
        ),
        Err(MergeError::UserApprovalRequired)
    ));

    fs::write(repo_path.join("other.txt"), b"target moved\n").expect("target move");
    git(&repo_path, &["add", "other.txt"]);
    git(&repo_path, &["commit", "-qm", "target moved"]);
    let mut user_approval = approval(&preview, MergeActor::User);
    assert!(matches!(
        ApprovedMerge::execute(
            &runner(&repo_path),
            &preview,
            &mut user_approval,
            request(&preview, MergeStrategy::Squash),
        ),
        Err(MergeError::TargetHeadChanged { .. })
    ));
    assert!(!user_approval.is_consumed());
    assert_eq!(
        fs::read_to_string(repo_path.join("tracked.txt")).expect("target"),
        "baseline\n"
    );
}

#[test]
fn conflict_preserves_both_sides_and_pauses_without_aborting_worktree() {
    let temp = tempfile::tempdir().expect("temp");
    let repo_path = temp.path().join("repo");
    fs::create_dir(&repo_path).expect("repo dir");
    repo(&repo_path, true);
    let store = EncryptedBlobStore::open(temp.path().join("blobs"), [13; 32]).expect("store");
    let preview = preview(&repo_path, &store);
    assert!(preview.conflict_precheck.has_conflicts);
    let mut approval = approval(&preview, MergeActor::User);

    let outcome = ApprovedMerge::execute(
        &runner(&repo_path),
        &preview,
        &mut approval,
        request(&preview, MergeStrategy::PreserveCheckpointHistory),
    )
    .expect("conflict is a paused outcome");

    let MergeOutcome::PausedForConflict { evidence } = outcome else {
        panic!("expected conflict pause");
    };
    assert!(approval.is_consumed());
    assert!(evidence.unmerged_paths.contains(&"tracked.txt".to_owned()));
    assert!(git(&repo_path, &["status", "--porcelain"]).contains("UU tracked.txt"));
    let conflicted = fs::read_to_string(repo_path.join("tracked.txt")).expect("conflicted file");
    assert!(conflicted.contains("target change"));
    assert!(conflicted.contains("route change"));
    assert!(
        git(&repo_path, &["show-ref", "--verify", "refs/heads/route"]).contains("refs/heads/route")
    );
}
