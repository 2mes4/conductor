//! `/skills` checkout — the agent tools repository.
//!
//! This is downloaded independently of the client's project code, enabling
//! **Dual-Checkout**: you can update agent tools without touching the client's
//! repository.

use std::path::Path;

use git2::Repository;

use crate::error::Result;

/// Clone or update the skills/tools repository at `/skills`.
///
/// Unlike `/target`, the skills repo is treated as ephemeral read-only
/// tooling — if it already exists we simply pull the latest changes.
pub fn prepare_skills(path: &Path, skills_repo_url: &str) -> Result<Repository> {
    if path.exists() && path.join(".git").exists() {
        tracing::info!(?path, "skills exist — pulling latest");
        let repo = Repository::open(path)?;
        {
            let mut remote = repo.find_remote("origin")?;
            remote.fetch(&["refs/heads/*:refs/heads/*"], None, None)?;
        }

        {
            let head = repo.head()?;
            let head_commit = head.peel_to_commit()?;
            repo.reset(head_commit.as_object(), git2::ResetType::Hard, None)?;
        }
        Ok(repo)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        tracing::info!(?path, "skills missing — cloning");
        let repo = Repository::clone(skills_repo_url, path)?;
        Ok(repo)
    }
}
