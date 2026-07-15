//! Smart MCP pre-indexing — detect repo size and pre-build the knowledge graph
//! during the checkout phase (Layer 2) so the graph is ready before the agent
//! starts.
//!
//! Strategy:
//! 1. After `prepare_target()`, count files and estimate repo size.
//! 2. If repo is large (> threshold), force persistent cache mode.
//! 3. Check the last-indexed commit SHA in `.mcp_cache/meta.json`.
//! 4. If SHA matches → skip re-indexing (graph is still valid).
//! 5. If SHA differs → trigger incremental re-index in the background.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::mcp::CacheMode;

/// Threshold: repos with more than this many tracked files use persistent cache.
const LARGE_REPO_FILE_THRESHOLD: usize = 5_000;

/// Metadata stored alongside the MCP cache for invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    pub indexed_commit_sha: String,
    pub indexed_at: chrono::DateTime<chrono::Utc>,
    pub file_count: usize,
    pub total_size_bytes: u64,
}

/// Analysis of the target repository.
#[derive(Debug, Clone)]
pub struct RepoAnalysis {
    pub file_count: usize,
    pub total_size_bytes: u64,
    pub head_sha: String,
    pub is_large: bool,
}

/// Analyze the target repository to determine indexing strategy.
pub fn analyze_repo(repo: &git2::Repository) -> Result<RepoAnalysis> {
    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;
    let head_sha = head_commit.id().to_string();

    // Count files by walking the tree.
    let tree = head_commit.tree()?;
    let mut file_count = 0usize;

    tree.walk(git2::TreeWalkMode::PreOrder, |_root, entry| {
        if let Some(git2::ObjectType::Blob) = entry.kind() {
            file_count += 1;
        }
        git2::TreeWalkResult::Ok
    })?;

    // Estimate: average blob size ~2KB for code.
    let total_size_bytes = (file_count as u64) * 2048;

    let is_large = file_count > LARGE_REPO_FILE_THRESHOLD;

    tracing::info!(
        head_sha = %head_sha,
        file_count,
        total_size_mb = total_size_bytes / 1_048_576,
        is_large,
        "repo analyzed"
    );

    Ok(RepoAnalysis {
        file_count,
        total_size_bytes,
        head_sha,
        is_large,
    })
}

/// Decide the cache mode based on repo analysis and existing cache state.
///
/// - Large repos always use persistent mode.
/// - Small repos default to ephemeral unless a cache already exists.
pub fn decide_cache_mode(analysis: &RepoAnalysis, cache_exists: bool) -> CacheMode {
    if analysis.is_large || cache_exists {
        CacheMode::Persistent
    } else {
        CacheMode::Ephemeral
    }
}

/// Read the cache metadata from disk (if it exists).
pub fn read_cache_meta(cache_dir: &Path) -> Result<Option<CacheMeta>> {
    let meta_path = cache_dir.join("meta.json");
    if !meta_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&meta_path)?;
    let meta: CacheMeta = serde_json::from_str(&content)?;
    Ok(Some(meta))
}

/// Write cache metadata after indexing.
pub fn write_cache_meta(cache_dir: &Path, meta: &CacheMeta) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;
    let meta_path = cache_dir.join("meta.json");
    let content = serde_json::to_string_pretty(meta)?;
    std::fs::write(&meta_path, content)?;
    tracing::debug!(cache_dir = %cache_dir.display(), "cache meta written");
    Ok(())
}

/// Check whether the existing cache is still valid for the current HEAD.
///
/// Returns `true` if the cache matches the current commit (no re-index needed).
pub fn is_cache_valid(analysis: &RepoAnalysis, cache_dir: &Path) -> Result<bool> {
    match read_cache_meta(cache_dir)? {
        Some(meta) => {
            let valid = meta.indexed_commit_sha == analysis.head_sha;
            if valid {
                tracing::info!(
                    cached_sha = %meta.indexed_commit_sha,
                    "MCP cache is valid — skipping re-index"
                );
            }
            Ok(valid)
        }
        None => Ok(false),
    }
}

/// Compute the default cache path for a workspace.
pub fn cache_path(workspace_root: &str, tenant: &str, project: &str) -> PathBuf {
    PathBuf::from(workspace_root)
        .join(tenant)
        .join(project)
        .join(".mcp_cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn decide_cache_mode_small_repo_no_cache() {
        let analysis = RepoAnalysis {
            file_count: 100,
            total_size_bytes: 1024,
            head_sha: "abc".into(),
            is_large: false,
        };
        assert_eq!(decide_cache_mode(&analysis, false), CacheMode::Ephemeral);
    }

    #[test]
    fn decide_cache_mode_small_repo_with_cache() {
        let analysis = RepoAnalysis {
            file_count: 100,
            total_size_bytes: 1024,
            head_sha: "abc".into(),
            is_large: false,
        };
        assert_eq!(decide_cache_mode(&analysis, true), CacheMode::Persistent);
    }

    #[test]
    fn decide_cache_mode_large_repo() {
        let analysis = RepoAnalysis {
            file_count: 10_000,
            total_size_bytes: 100_000_000,
            head_sha: "abc".into(),
            is_large: true,
        };
        assert_eq!(decide_cache_mode(&analysis, false), CacheMode::Persistent);
    }

    #[test]
    fn read_cache_meta_returns_none_if_missing() {
        let tmp = TempDir::new().unwrap();
        let meta = read_cache_meta(tmp.path()).unwrap();
        assert!(meta.is_none());
    }

    #[test]
    fn write_and_read_cache_meta() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join(".mcp_cache");

        let meta = CacheMeta {
            indexed_commit_sha: "abc123".into(),
            indexed_at: chrono::Utc::now(),
            file_count: 500,
            total_size_bytes: 1_048_576,
        };

        write_cache_meta(&cache_dir, &meta).unwrap();
        let read = read_cache_meta(&cache_dir).unwrap().unwrap();
        assert_eq!(read.indexed_commit_sha, "abc123");
        assert_eq!(read.file_count, 500);
    }

    #[test]
    fn is_cache_valid_when_sha_matches() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join(".mcp_cache");

        let meta = CacheMeta {
            indexed_commit_sha: "abc123".into(),
            indexed_at: chrono::Utc::now(),
            file_count: 500,
            total_size_bytes: 1024,
        };
        write_cache_meta(&cache_dir, &meta).unwrap();

        let analysis = RepoAnalysis {
            file_count: 500,
            total_size_bytes: 1024,
            head_sha: "abc123".into(),
            is_large: false,
        };

        assert!(is_cache_valid(&analysis, &cache_dir).unwrap());
    }

    #[test]
    fn is_cache_invalid_when_sha_differs() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join(".mcp_cache");

        let meta = CacheMeta {
            indexed_commit_sha: "old_sha".into(),
            indexed_at: chrono::Utc::now(),
            file_count: 500,
            total_size_bytes: 1024,
        };
        write_cache_meta(&cache_dir, &meta).unwrap();

        let analysis = RepoAnalysis {
            file_count: 500,
            total_size_bytes: 1024,
            head_sha: "new_sha".into(),
            is_large: false,
        };

        assert!(!is_cache_valid(&analysis, &cache_dir).unwrap());
    }

    #[test]
    fn cache_path_is_project_level() {
        let path = cache_path("/workspace", "acme", "webapp");
        assert_eq!(path, PathBuf::from("/workspace/acme/webapp/.mcp_cache"));
    }
}
