//! Persistence — commit changes, store session, and clean up.
//!
//! Using `git2`, check the working tree of `/target`. If there are changes,
//! stage them, create a commit, and push to the original branch.

use git2::Repository;

use crate::error::Result;

/// Default commit message for agent-produced changes.
pub const DEFAULT_COMMIT_MSG: &str = "chore(ai): updates by agent";

/// Commit and push any changes left in the target working tree.
///
/// Delegates to [`crate::checkout::target::commit_and_push`].
pub fn save_changes(repo: &Repository, branch: &str, message: &str) -> Result<Option<String>> {
    crate::checkout::target::commit_and_push(repo, branch, message)
}

/// Remove the ephemeral workspace directory after teardown.
pub fn cleanup_workspace(workspace_root: &std::path::Path) -> Result<()> {
    if workspace_root.exists() {
        tracing::info!(?workspace_root, "cleaning up workspace");
        std::fs::remove_dir_all(workspace_root)?;
    }
    Ok(())
}
