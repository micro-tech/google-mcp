//! Configuration for google-mcp.
//!
//! Everything comes from the environment — there are no secrets in files,
//! flags, or the repo. The only file ever written is the OAuth token cache,
//! whose path is itself configurable.

use std::collections::HashMap;
use std::path::PathBuf;

/// How the first-run OAuth consent flow returns the authorization code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthMethod {
    /// Spin up a localhost HTTP server; Google redirects the browser back to
    /// it. This is the method Google supports for "Desktop app" OAuth clients
    /// (the old copy-paste PIN / OOB flow was removed by Google).
    HttpRedirect,
    /// Print a URL and have the user paste back an authorization code.
    /// Kept for completeness; rarely works with new Google clients.
    Interactive,
}

impl OAuthMethod {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "interactive" | "pin" => OAuthMethod::Interactive,
            _ => OAuthMethod::HttpRedirect,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    /// OAuth 2.0 Client ID (Google Cloud Console → Credentials).
    pub client_id: String,
    /// OAuth 2.0 Client secret.
    pub client_secret: String,
    /// Where the OAuth refresh token is cached after the first `auth` run.
    pub token_cache: PathBuf,
    /// Which installed-app flow to use on first run.
    pub oauth_method: OAuthMethod,
}

impl GoogleConfig {
    /// Read configuration from the process environment.
    pub fn from_env() -> anyhow::Result<Self> {
        let vars: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&vars)
    }

    /// Build configuration from an explicit map (used by tests so they don't
    /// have to mutate the real process environment).
    pub fn from_map(vars: &HashMap<String, String>) -> anyhow::Result<Self> {
        let get = |key: &str| {
            vars.get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "{key} is not set. Export it before running google-mcp (see README.md)."
                    )
                })
        };

        let token_cache = match vars.get("GOOGLE_TOKEN_CACHE") {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
            _ => default_token_cache()?,
        };

        let oauth_method = vars
            .get("GOOGLE_OAUTH_METHOD")
            .map(|s| OAuthMethod::parse(s))
            .unwrap_or(OAuthMethod::HttpRedirect);

        Ok(Self {
            client_id: get("GOOGLE_CLIENT_ID")?,
            client_secret: get("GOOGLE_CLIENT_SECRET")?,
            token_cache,
            oauth_method,
        })
    }
}

fn default_token_cache() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("google-mcp").join("token.json"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine a config directory; set GOOGLE_TOKEN_CACHE explicitly"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_full_config() {
        let cfg = GoogleConfig::from_map(&vars(&[
            ("GOOGLE_CLIENT_ID", "id123"),
            ("GOOGLE_CLIENT_SECRET", "secret456"),
            ("GOOGLE_TOKEN_CACHE", "/tmp/tok.json"),
            ("GOOGLE_OAUTH_METHOD", "interactive"),
        ]))
        .unwrap();
        assert_eq!(cfg.client_id, "id123");
        assert_eq!(cfg.client_secret, "secret456");
        assert_eq!(cfg.token_cache, PathBuf::from("/tmp/tok.json"));
        assert_eq!(cfg.oauth_method, OAuthMethod::Interactive);
    }

    #[test]
    fn defaults_to_http_redirect_and_config_dir() {
        let cfg = GoogleConfig::from_map(&vars(&[
            ("GOOGLE_CLIENT_ID", "id"),
            ("GOOGLE_CLIENT_SECRET", "secret"),
        ]))
        .unwrap();
        assert_eq!(cfg.oauth_method, OAuthMethod::HttpRedirect);
        assert!(cfg.token_cache.ends_with("google-mcp/token.json"));
    }

    #[test]
    fn missing_client_id_is_an_error() {
        let err = GoogleConfig::from_map(&vars(&[("GOOGLE_CLIENT_SECRET", "secret")]))
            .unwrap_err();
        assert!(err.to_string().contains("GOOGLE_CLIENT_ID"));
    }

    #[test]
    fn missing_client_secret_is_an_error() {
        let err = GoogleConfig::from_map(&vars(&[("GOOGLE_CLIENT_ID", "id")])).unwrap_err();
        assert!(err.to_string().contains("GOOGLE_CLIENT_SECRET"));
    }

    #[test]
    fn blank_values_are_rejected() {
        let err = GoogleConfig::from_map(&vars(&[
            ("GOOGLE_CLIENT_ID", "   "),
            ("GOOGLE_CLIENT_SECRET", "secret"),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("GOOGLE_CLIENT_ID"));
    }
}
