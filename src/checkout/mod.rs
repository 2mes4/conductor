//! Layer 2 — Dual-Checkout Engine.
//!
//! Manages the filesystem layout for agent workspaces. Rust manipulates Git
//! directly in-memory via `git2` (never shell calls) for security.
//!
//! ## Directory layout
//!
//! ```text
//! /workspace/{tenant}/{project}/
//! ├── .mcp_cache/              ← persistent MCP graph (project-level, shared across branches)
//! └── {branch}/
//!     ├── target/   ← business code (read-write)
//!     └── skills/   ← agent tools   (read-only)
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
    /// Persistent MCP cache directory (project-level, shared across branches).
    pub mcp_cache: PathBuf,
}

impl Workspace {
    /// Construct the workspace path for a given tenant/project/branch triple.
    pub fn new(workspace_root: &str, tenant_slug: &str, project_slug: &str, branch: &str) -> Self {
        let root = PathBuf::from(workspace_root)
            .join(tenant_slug)
            .join(project_slug)
            .join(branch);

        let mcp_cache = PathBuf::from(workspace_root)
            .join(tenant_slug)
            .join(project_slug)
            .join(".mcp_cache");

        Self {
            target: root.join("target"),
            skills: root.join("skills"),
            mcp_cache,
            root,
        }
    }

    /// Ensure the root workspace directory exists.
    pub fn ensure_root(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Ensure the MCP cache directory exists.
    pub fn ensure_mcp_cache(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.mcp_cache)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_paths_are_correct() {
        let ws = Workspace::new("/workspace", "acme", "webapp", "feature/auth");
        assert_eq!(
            ws.root,
            PathBuf::from("/workspace/acme/webapp/feature/auth")
        );
        assert_eq!(
            ws.target,
            PathBuf::from("/workspace/acme/webapp/feature/auth/target")
        );
        assert_eq!(
            ws.skills,
            PathBuf::from("/workspace/acme/webapp/feature/auth/skills")
        );
    }

    #[test]
    fn mcp_cache_is_project_level() {
        let ws1 = Workspace::new("/workspace", "acme", "webapp", "feature/a");
        let ws2 = Workspace::new("/workspace", "acme", "webapp", "feature/b");
        assert_eq!(ws1.mcp_cache, ws2.mcp_cache);
        assert_eq!(
            ws1.mcp_cache,
            PathBuf::from("/workspace/acme/webapp/.mcp_cache")
        );
    }
}
