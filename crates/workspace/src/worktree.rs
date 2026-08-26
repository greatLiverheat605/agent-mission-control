use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::git::{GitError, GitRunner};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaselineSelection {
    CommittedHead(String),
    SnapshotCommit(String),
}

impl BaselineSelection {
    fn commit(&self) -> &str {
        match self {
            Self::CommittedHead(commit) | Self::SnapshotCommit(commit) => commit,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteWorkspace {
    pub branch: String,
    pub path: PathBuf,
    pub baseline_commit: String,
}

#[derive(Clone, Debug)]
pub struct RouteWorkspaceManager {
    runner: GitRunner,
    managed_root: PathBuf,
}

impl RouteWorkspaceManager {
    pub fn new(runner: GitRunner, managed_root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        fs::create_dir_all(managed_root.as_ref()).map_err(WorkspaceError::Io)?;
        let managed_root = managed_root
            .as_ref()
            .canonicalize()
            .map_err(WorkspaceError::Io)?;
        Ok(Self {
            runner,
            managed_root,
        })
    }

    pub fn create(
        &self,
        mission_id: &str,
        route_id: &str,
        baseline: BaselineSelection,
    ) -> Result<RouteWorkspace, WorkspaceError> {
        let mission_short = short_id(mission_id)?;
        let route_short = short_id(route_id)?;
        let branch = format!("mission/{mission_short}/{route_short}");
        let target = self.managed_root.join(&mission_short).join(&route_short);
        if !target.starts_with(&self.managed_root) {
            return Err(WorkspaceError::OutsideManagedRoot(target));
        }
        let target_existed = target.exists();
        if target_existed
            && fs::read_dir(&target)
                .map_err(WorkspaceError::Io)?
                .next()
                .is_some()
        {
            return Err(WorkspaceError::TargetNotEmpty(target));
        }
        if self.branch_exists(&branch)? {
            return Err(WorkspaceError::BranchExists(branch));
        }

        let selected = baseline.commit().trim().to_owned();
        if selected.is_empty() {
            return Err(WorkspaceError::InvalidBaseline);
        }
        self.runner
            .run_text(&["cat-file", "-e", &format!("{selected}^{{commit}}")])?;
        if matches!(baseline, BaselineSelection::CommittedHead(_)) {
            let head = self
                .runner
                .run_text(&["rev-parse", "--verify", "HEAD"])?
                .stdout
                .trim()
                .to_owned();
            if head != selected {
                return Err(WorkspaceError::InvalidBaseline);
            }
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(WorkspaceError::Io)?;
        }
        let args = vec![
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(&branch),
            git_cli_path(&target).into_os_string(),
            OsString::from(&selected),
        ];
        if let Err(error) = self.runner.run(&args) {
            self.cleanup_failed_create(&branch, &target, target_existed);
            return Err(WorkspaceError::Git(error));
        }
        let path = target.canonicalize().map_err(WorkspaceError::Io)?;
        Ok(RouteWorkspace {
            branch,
            path,
            baseline_commit: selected,
        })
    }

    fn branch_exists(&self, branch: &str) -> Result<bool, WorkspaceError> {
        let output = self.runner.run_unchecked(&[
            OsString::from("show-ref"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from(format!("refs/heads/{branch}")),
        ])?;
        match output.status {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(WorkspaceError::Git(GitError::CommandFailed(output))),
        }
    }

    fn cleanup_failed_create(&self, branch: &str, target: &Path, target_existed: bool) {
        if self.branch_exists(branch).unwrap_or(false) {
            let _ = self.runner.run(&[
                OsString::from("branch"),
                OsString::from("-D"),
                OsString::from(branch),
            ]);
        }
        if target.starts_with(&self.managed_root) && target.exists() {
            let _ = fs::remove_dir_all(target);
            if target_existed {
                let _ = fs::create_dir_all(target);
            }
        }
    }
}

fn git_cli_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if let Some(stripped) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{stripped}"));
        }
        if let Some(stripped) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    path.to_owned()
}

fn short_id(value: &str) -> Result<String, WorkspaceError> {
    let short: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if short.len() != 8 {
        return Err(WorkspaceError::InvalidOpaqueId);
    }
    Ok(short.to_ascii_lowercase())
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("workspace I/O failed: {0}")]
    Io(std::io::Error),
    #[error("opaque Mission/Route id is invalid")]
    InvalidOpaqueId,
    #[error("baseline commit is invalid")]
    InvalidBaseline,
    #[error("branch already exists: {0}")]
    BranchExists(String),
    #[error("worktree target is not empty: {0}")]
    TargetNotEmpty(PathBuf),
    #[error("worktree target is outside managed root: {0}")]
    OutsideManagedRoot(PathBuf),
}
