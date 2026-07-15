//! `/target` checkout — the business code repository.
//!
//! Rust checks whether `/workspace/{tenant}/{project}/{branch}/target` exists.
//! If not, it clones the repository; if it does, it forces a `git fetch` and
//! `git reset --hard` to guarantee a clean state before the agent touches it.

use std::path::Path;

use git2::{BranchType, Repository, ResetType};

use crate::error::{ConductorError, Result};

/// Prepare the `/target` working directory for an agent session.
///
/// - If the directory doesn't exist → clone the repo at the specified branch.
/// - If it exists → fetch + hard reset to the remote branch for a clean slate.
///
/// Returns the [`Repository`] handle for further Git operations.
pub fn prepare_target(
    path: &Path,
    repo_url: &str,
    branch: &str,
) -> Result<Repository> {
    if path.exists() && path.join(".git").exists() {
        tracing::info!(?path, "target exists — fetching and resetting");
        sync_existing(path, branch)
    } else {
        tracing::info!(?path, "target missing — cloning");
        clone_fresh(path, repo_url, branch)
    }
}

fn clone_fresh(path: &Path, repo_url: &str, branch: &str) -> Result<Repository> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut builder = git2::build::RepoBuilder::new();
    builder.branch(branch);

    let repo = builder.clone(repo_url, path)?;
    tracing::info!(?path, branch, "cloned target repository");
    Ok(repo)
}

fn sync_existing(path: &Path, branch: &str) -> Result<Repository> {
    let repo = Repository::open(path)?;

    // Fetch all remotes.
    let mut remote = repo.find_remote("origin")?;
    remote.fetch(&[branch], None, None)?;

    // Resolve the remote branch ref: refs/remotes/origin/{branch}.
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let fetch_head = repo.find_reference(&remote_ref)?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;

    // Hard reset the working tree to match the remote.
    let target = repo.find_commit(fetch_commit.id())?;
    repo.reset(target.as_object(), ResetType::Hard, None)?;

    // Check out the local branch (create it if missing).
    if repo.find_branch(branch, BranchType::Local).is_err() {
        let upstream = repo.find_branch(&format!("origin/{branch}"), BranchType::Remote)?;
        let upstream_commit = upstream.get().peel_to_commit()?;
        repo.branch(branch, &upstream_commit, false)?;
    }
    repo.set_head(&format!("refs/heads/{branch}"))?;
    repo.checkout_head(None)?;

    tracing::info!(?path, branch, "target synced (fetch + hard reset)");
    Ok(repo)
}

/// Commit and push any changes in the `/target` working tree after the agent
/// session completes.
pub fn commit_and_push(repo: &Repository, branch: &str, message: &str) -> Result<Option<String>> {
    // Stage all changes.
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head = repo.head()?;
    let parent_commit = head.peel_to_commit()?;

    // Only commit if there are actual changes.
    let diff = repo.diff_tree_to_workdir_with_index(
        Some(&parent_commit.tree()?, None),
        None,
    )?;

    if diff.stats()?.files_changed() == 0 {
        tracing::info!("no changes to commit in target");
        return Ok(None);
    }

    let sig = repo.signature()?;
    let commit_id = repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        message,
        &tree,
        &[&parent_commit],
    )?;

    // Push to the remote branch.
    let mut remote = repo.find_remote("origin")?;
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    remote.push(&[&refspec], None)?;

    let sha = commit_id.to_string();
    tracing::info!(sha = %sha, "committed and pushed target changes");
    Ok(Some(sha))
}
