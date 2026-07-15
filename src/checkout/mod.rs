//! Layer 2 — Dual-Checkout Engine.
//!
//! Manages the filesystem layout for agent workspaces. Rust manipulates Git
//! directly in-memory via `git2` (never shell calls) for security.
//!
//! ## Directory layout
//!
//! ```text
//! /workspace/{tenant}/{project}/{branch}/
//! ├── target/   ← business code (read-write)
//! └── skills/   ← agent tools   (read-only)
//! ```
//!
//! - [`target`] — mount the `/target` checkout (clone or fetch + reset)
//! - [`skills`] — mount the `/skills` repository (independent of client code)

pub mod skills;
pub mod target;

use std::path::PathBuf;

/// A fully prepared workspace directory.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub target: PathBuf,
    pub skills: PathBuf,
}

impl Workspace {
    /// Construct the workspace path for a given tenant/project/branch triple.
    pub fn new(
        workspace_root: &str,
        tenant_slug: &str,
        project_slug: &str,
        branch: &str,
    ) -> Self {
        let root = PathBuf::from(workspace_root)
            .join(tenant_slug)
            .join(project_slug)
            .join(branch);

        Self {
            target: root.join("target"),
            skills: root.join("skills"),
            root,
        }
    }

    /// Ensure the root workspace directory exists.
    pub fn ensure_root(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }
}
