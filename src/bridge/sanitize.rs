//! Path sanitisation — ensures tool executables don't contain directory
//! traversal sequences (e.g. `../`).

use crate::error::{ConductorError, Result};

/// Validate that a path does not escape its intended directory.
pub fn validate_path(path: &str) -> Result<()> {
    if path.contains("..") {
        return Err(ConductorError::PathTraversal(path.to_string()));
    }
    if path.starts_with('/') {
        return Err(ConductorError::PathTraversal(format!(
            "absolute path not allowed: {path}"
        )));
    }
    Ok(())
}

/// Sanitise a path by normalising it and checking for traversal.
/// Returns the cleaned path on success.
pub fn sanitize_path(path: &str) -> Result<String> {
    validate_path(path)?;

    let parts: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();

    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        assert!(validate_path("../../../etc/passwd").is_err());
        assert!(validate_path("tools/../escape").is_err());
    }

    #[test]
    fn rejects_absolute() {
        assert!(validate_path("/bin/sh").is_err());
    }

    #[test]
    fn accepts_clean_relative() {
        assert!(validate_path("tools/lint.sh").is_ok());
    }

    #[test]
    fn sanitizes_dots() {
        assert_eq!(sanitize_path("./tools//lint.sh").unwrap(), "tools/lint.sh");
    }
}
