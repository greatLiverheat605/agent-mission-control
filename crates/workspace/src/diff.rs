use std::ffi::OsString;

use mission_ledger::{BlobRef, BlobStoreError, EncryptedBlobStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{GitError, GitRunner};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPreviewRequest {
    pub target_branch: String,
    pub source_branch: String,
    pub validation_commands: Vec<String>,
    pub risks: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConflictPrecheck {
    pub has_conflicts: bool,
    pub details: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffPreview {
    pub target_branch: String,
    pub source_branch: String,
    pub target_head: String,
    pub source_head: String,
    pub commit_range: String,
    pub stat: String,
    pub diff_blob: BlobRef,
    pub validation_commands: Vec<String>,
    pub risks: Vec<String>,
    pub conflict_precheck: ConflictPrecheck,
}

impl DiffPreview {
    pub fn create(
        runner: &GitRunner,
        store: &EncryptedBlobStore,
        request: DiffPreviewRequest,
    ) -> Result<Self, DiffPreviewError> {
        validate_branch(runner, &request.target_branch)?;
        validate_branch(runner, &request.source_branch)?;
        if request
            .validation_commands
            .iter()
            .any(|value| value.trim().is_empty())
            || request.risks.iter().any(|value| value.trim().is_empty())
        {
            return Err(DiffPreviewError::IncompleteMetadata);
        }

        let target_head = resolve_head(runner, &request.target_branch)?;
        let source_head = resolve_head(runner, &request.source_branch)?;
        let stat = runner
            .run_text(&["diff", "--stat", &target_head, &source_head])?
            .stdout;
        let full_diff = runner
            .run_text(&[
                "diff",
                "--binary",
                "--full-index",
                &target_head,
                &source_head,
            ])?
            .stdout;
        let conflict_precheck = conflict_precheck(runner, &target_head, &source_head)?;
        let diff_blob = store.put(full_diff.as_bytes(), "text/x-diff")?;
        store.retain(&diff_blob)?;

        Ok(Self {
            target_branch: request.target_branch,
            source_branch: request.source_branch,
            commit_range: format!("{target_head}..{source_head}"),
            target_head,
            source_head,
            stat,
            diff_blob,
            validation_commands: request.validation_commands,
            risks: request.risks,
            conflict_precheck,
        })
    }

    pub fn digest(&self) -> String {
        let encoded = serde_json::to_vec(self).expect("diff preview is serializable");
        Sha256::digest(encoded)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

pub(crate) fn resolve_head(runner: &GitRunner, branch: &str) -> Result<String, GitError> {
    Ok(runner
        .run_text(&[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ])?
        .stdout
        .trim()
        .to_owned())
}

fn validate_branch(runner: &GitRunner, branch: &str) -> Result<(), DiffPreviewError> {
    if branch.trim().is_empty() {
        return Err(DiffPreviewError::InvalidBranch);
    }
    let output = runner.run_unchecked(&[
        OsString::from("check-ref-format"),
        OsString::from("--branch"),
        OsString::from(branch),
    ])?;
    if output.status != 0 {
        return Err(DiffPreviewError::InvalidBranch);
    }
    Ok(())
}

fn conflict_precheck(
    runner: &GitRunner,
    target_head: &str,
    source_head: &str,
) -> Result<ConflictPrecheck, DiffPreviewError> {
    let output = runner.run_unchecked(&[
        OsString::from("merge-tree"),
        OsString::from("--write-tree"),
        OsString::from(target_head),
        OsString::from(source_head),
    ])?;
    match output.status {
        0 | 1 => Ok(ConflictPrecheck {
            has_conflicts: output.status == 1,
            details: format!("{}{}", output.stdout, output.stderr),
        }),
        _ => Err(GitError::CommandFailed(output).into()),
    }
}

#[derive(Debug, Error)]
pub enum DiffPreviewError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Blob(#[from] BlobStoreError),
    #[error("Git branch name is invalid")]
    InvalidBranch,
    #[error("diff preview validation commands or risks are incomplete")]
    IncompleteMetadata,
}
