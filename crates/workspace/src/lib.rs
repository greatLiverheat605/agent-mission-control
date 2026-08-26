mod checkpoint;
mod diff;
mod git;
mod merge;
mod snapshot;
mod worktree;

pub use checkpoint::{
    CheckpointError, CheckpointMetadata, CheckpointRequest, CheckpointTrigger, GitCheckpoint,
};
pub use diff::{ConflictPrecheck, DiffPreview, DiffPreviewError, DiffPreviewRequest};
pub use git::{BaselineState, GitError, GitOutput, GitRunner, inspect_baseline};
pub use merge::{
    ApprovedMerge, MergeActor, MergeApproval, MergeConflictEvidence, MergeError, MergeOutcome,
    MergeRequest, MergeStrategy,
};
pub use snapshot::{
    NonGitSnapshot, NonGitSnapshotter, PreparedRestore, SnapshotEntry, SnapshotError,
    SnapshotManifest, SnapshotOptions,
};
pub use worktree::{BaselineSelection, RouteWorkspace, RouteWorkspaceManager, WorkspaceError};
