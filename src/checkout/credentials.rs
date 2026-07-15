//! Git credential callbacks for private repositories.
//!
//! Injects ephemeral tokens into `git2` RemoteCallbacks so that private repos
//! can be cloned/fetched without hardcoding secrets.

use git2::{CredentialType, RemoteCallbacks};

/// Credentials provider for Git operations.
#[derive(Clone)]
pub struct GitCredentials {
    /// HTTPS username (typically the token name or "x-access-token").
    pub username: Option<String>,
    /// HTTPS password / personal access token.
    #[allow(dead_code)]
    pub token: Option<String>,
    /// Path to SSH private key.
    pub ssh_key_path: Option<String>,
}

impl GitCredentials {
    /// Create credentials for HTTPS token auth.
    pub fn https_token(username: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            username: Some(username.into()),
            token: Some(token.into()),
            ssh_key_path: None,
        }
    }

    /// Create credentials for SSH key auth.
    pub fn ssh_key(path: impl Into<String>) -> Self {
        Self {
            username: Some("git".into()),
            token: None,
            ssh_key_path: Some(path.into()),
        }
    }

    /// No credentials (public repos).
    pub fn none() -> Self {
        Self {
            username: None,
            token: None,
            ssh_key_path: None,
        }
    }

    /// Load credentials from environment variables.
    pub fn from_env() -> Self {
        if let (Ok(user), Ok(token)) = (std::env::var("GIT_USERNAME"), std::env::var("GIT_TOKEN")) {
            Self::https_token(user, token)
        } else if let Ok(key) = std::env::var("GIT_SSH_KEY") {
            Self::ssh_key(key)
        } else {
            Self::none()
        }
    }

    /// Build a `RemoteCallbacks` with these credentials.
    pub fn build_callbacks(&self) -> RemoteCallbacks<'_> {
        let mut callbacks = RemoteCallbacks::new();
        let username = self.username.clone();
        let ssh_key = self.ssh_key_path.clone();
        let token_ref = self.token.as_deref();

        callbacks.credentials(move |url, username_from_url, allowed| {
            let _ = url;

            if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(token) = token_ref {
                    let user = username_from_url.unwrap_or(username.as_deref().unwrap_or("git"));
                    tracing::debug!(url, "using HTTPS token auth");
                    return git2::Cred::userpass_plaintext(user, token);
                }
            }

            if allowed.contains(CredentialType::SSH_KEY) {
                if let Some(key_path) = &ssh_key {
                    let user = username_from_url.unwrap_or("git");
                    tracing::debug!("using SSH key auth");
                    return git2::Cred::ssh_key(user, None, std::path::Path::new(key_path), None);
                }
            }

            Err(git2::Error::from_str("no suitable credentials available"))
        });

        callbacks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_token_credentials() {
        let creds = GitCredentials::https_token("user", "token123");
        assert_eq!(creds.username.as_deref(), Some("user"));
        assert_eq!(creds.token.as_deref(), Some("token123"));
        assert!(creds.ssh_key_path.is_none());
    }

    #[test]
    fn ssh_key_credentials() {
        let creds = GitCredentials::ssh_key("/home/user/.ssh/id_rsa");
        assert_eq!(creds.username.as_deref(), Some("git"));
        assert!(creds.ssh_key_path.is_some());
        assert!(creds.token.is_none());
    }

    #[test]
    fn none_credentials() {
        let creds = GitCredentials::none();
        assert!(creds.username.is_none());
        assert!(creds.token.is_none());
        assert!(creds.ssh_key_path.is_none());
    }

    #[test]
    fn build_callbacks_returns_valid_callbacks() {
        let creds = GitCredentials::https_token("user", "token");
        let _callbacks = creds.build_callbacks();
    }

    #[test]
    fn from_env_falls_back_to_none() {
        std::env::remove_var("GIT_USERNAME");
        std::env::remove_var("GIT_TOKEN");
        std::env::remove_var("GIT_SSH_KEY");
        let creds = GitCredentials::from_env();
        assert!(creds.username.is_none());
    }
}
