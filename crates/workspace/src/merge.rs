use std::ffi::OsString;
use std::path::PathBuf;

use mission_domain::RouteState;
use thiserror::Error;

use crate::diff::resolve_head;
use crate::{DiffPreview, GitError, GitRunner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeActor {
    User,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeStrategy {
    PreserveCheckpointHistory,
    Squash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeApproval {
    approval_id: String,
    actor: MergeActor,
    preview_digest: String,
    expected_target_head: String,
    consumed: bool,
}

impl MergeApproval {
    pub fn new(
        approval_id: impl Into<String>,
        actor: MergeActor,
        preview_digest: impl Into<String>,
        expected_target_head: impl Into<String>,
    ) -> Result<Self, MergeError> {
        let approval_id = approval_id.into();
        let preview_digest = preview_digest.into();
        let expected_target_head = expected_target_head.into();
        if approval_id.is_empty()
            || approval_id.len() > 64
            || !approval_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !valid_hex(&preview_digest, &[64])
            || !valid_hex(&expected_target_head, &[40, 64])
        {
            return Err(MergeError::InvalidApproval);
        }
        Ok(Self {
            approval_id,
            actor,
            preview_digest,
            expected_target_head,
            consumed: false,
        })
    }

    pub const fn is_consumed(&self) -> bool {
        self.consumed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeRequest {
    pub expected_target_head: String,
    pub strategy: MergeStrategy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeConflictEvidence {
    pub route_state: RouteState,
    pub reason: String,
    pub unmerged_paths: Vec<String>,
    pub worktree: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeOutcome {
    Merged { commit: String },
    PausedForConflict { evidence: MergeConflictEvidence },
}

pub struct ApprovedMerge;

impl ApprovedMerge {
    pub fn execute(
        runner: &GitRunner,
        preview: &DiffPreview,
        approval: &mut MergeApproval,
        request: MergeRequest,
    ) -> Result<MergeOutcome, MergeError> {
        validate_approval(preview, approval, &request)?;
        validate_target_worktree(runner, preview)?;
        let source_head = resolve_head(runner, &preview.source_branch)?;
        if source_head != preview.source_head {
            return Err(MergeError::SourceHeadChanged {
                expected: preview.source_head.clone(),
                actual: source_head,
            });
        }
        approval.consumed = true;

        let source_ref = format!("refs/heads/{}", preview.source_branch);
        let strategy_args: &[&str] = match request.strategy {
            MergeStrategy::PreserveCheckpointHistory => &["--no-ff", "--no-commit"],
            MergeStrategy::Squash => &["--squash", "--no-commit"],
        };
        let mut args: Vec<OsString> = [
            "-c",
            "user.name=Agent Mission Control",
            "-c",
            "user.email=merge@mission-control.invalid",
            "merge",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.extend(strategy_args.iter().map(OsString::from));
        args.push(OsString::from("--"));
        args.push(OsString::from(source_ref));
        let merge = runner.run_unchecked(&args)?;
        if merge.status != 0 {
            let unmerged = runner.run_text(&["diff", "--name-only", "--diff-filter=U"])?;
            let unmerged_paths: Vec<_> = unmerged
                .stdout
                .lines()
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect();
            if unmerged_paths.is_empty() {
                return Err(GitError::CommandFailed(merge).into());
            }
            return Ok(MergeOutcome::PausedForConflict {
                evidence: MergeConflictEvidence {
                    route_state: RouteState::Paused,
                    reason: format!("{}{}", merge.stdout, merge.stderr),
                    unmerged_paths,
                    worktree: runner.cwd().to_owned(),
                },
            });
        }

        runner.run(&[
            OsString::from("-c"),
            OsString::from("user.name=Agent Mission Control"),
            OsString::from("-c"),
            OsString::from("user.email=merge@mission-control.invalid"),
            OsString::from("commit"),
            OsString::from("-m"),
            OsString::from(format!(
                "Mission Control approved merge {}",
                approval.approval_id
            )),
        ])?;
        let commit = runner
            .run_text(&["rev-parse", "HEAD"])?
            .stdout
            .trim()
            .to_owned();
        Ok(MergeOutcome::Merged { commit })
    }
}

fn validate_approval(
    preview: &DiffPreview,
    approval: &MergeApproval,
    request: &MergeRequest,
) -> Result<(), MergeError> {
    if approval.consumed {
        return Err(MergeError::ApprovalConsumed);
    }
    if approval.actor != MergeActor::User {
        return Err(MergeError::UserApprovalRequired);
    }
    if approval.preview_digest != preview.digest()
        || approval.expected_target_head != preview.target_head
        || request.expected_target_head != preview.target_head
    {
        return Err(MergeError::ApprovalMismatch);
    }
    Ok(())
}

fn validate_target_worktree(runner: &GitRunner, preview: &DiffPreview) -> Result<(), MergeError> {
    let branch = runner
        .run_text(&["symbolic-ref", "--quiet", "--short", "HEAD"])?
        .stdout
        .trim()
        .to_owned();
    if branch != preview.target_branch {
        return Err(MergeError::WrongTargetBranch {
            expected: preview.target_branch.clone(),
            actual: branch,
        });
    }
    let actual = runner
        .run_text(&["rev-parse", "HEAD"])?
        .stdout
        .trim()
        .to_owned();
    if actual != preview.target_head {
        return Err(MergeError::TargetHeadChanged {
            expected: preview.target_head.clone(),
            actual,
        });
    }
    if !runner
        .run_text(&["status", "--porcelain=v1", "--untracked-files=all"])?
        .stdout
        .is_empty()
    {
        return Err(MergeError::TargetWorktreeDirty);
    }
    Ok(())
}

fn valid_hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Error)]
pub enum MergeError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("merge approval is malformed")]
    InvalidApproval,
    #[error("only a user can approve merge")]
    UserApprovalRequired,
    #[error("merge approval has already been consumed")]
    ApprovalConsumed,
    #[error("merge approval does not match the reviewed preview")]
    ApprovalMismatch,
    #[error("target HEAD changed from {expected} to {actual}; a new preview is required")]
    TargetHeadChanged { expected: String, actual: String },
    #[error("source HEAD changed from {expected} to {actual}; a new preview is required")]
    SourceHeadChanged { expected: String, actual: String },
    #[error("expected target branch {expected}, found {actual}")]
    WrongTargetBranch { expected: String, actual: String },
    #[error("target worktree must be clean before merge")]
    TargetWorktreeDirty,
}
