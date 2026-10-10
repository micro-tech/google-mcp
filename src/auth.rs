//! Google OAuth2 (installed-application flow) for google-mcp.
//!
//! The client ID/secret come from the environment (see [`config`]); the only
//! thing ever persisted is the refresh/access token pair, written to
//! `GOOGLE_TOKEN_CACHE` (default `~/.config/google-mcp/token.json`).

use crate::config::{GoogleConfig, OAuthMethod};
use yup_oauth2::{ApplicationSecret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

/// OAuth scopes requested on first authorization.
pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/calendar",
];

/// The TLS connector type yup-oauth2 builds internally (via its re-exported
/// hyper-rustls 0.27, which matches ours, so the types unify).
pub type GoogleConnector = yup_oauth2::hyper_rustls::HttpsConnector<
    hyper_util::client::legacy::connect::HttpConnector,
>;

/// Concrete yup-oauth2 authenticator type used by the API hubs.
///
/// Note: `InstalledFlowAuthenticator::builder(...).build().await` resolves the
/// builder into an `Authenticator` over the *connector*, not the builder.
pub type GoogleAuth = yup_oauth2::authenticator::Authenticator<GoogleConnector>;

/// Build an authenticator from environment config.
///
/// This performs no network I/O by itself: it loads the cached token from
/// disk when present. The browser consent flow only runs lazily, the first
/// time a token is actually requested (or eagerly via `google-mcp auth`).
pub async fn build_authenticator(config: &GoogleConfig) -> anyhow::Result<GoogleAuth> {
    if let Some(parent) = config.token_cache.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let secret = ApplicationSecret {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        auth_uri: "https://accounts.google.com/o/oauth2/auth".to_string(),
        // "Desktop app" OAuth clients use the loopback redirect.
        redirect_uris: vec!["http://localhost".to_string()],
        project_id: None,
        client_email: None,
        auth_provider_x509_cert_url: None,
        client_x509_cert_url: None,
    };

    let method = match config.oauth_method {
        OAuthMethod::HttpRedirect => InstalledFlowReturnMethod::HTTPRedirect,
        OAuthMethod::Interactive => InstalledFlowReturnMethod::Interactive,
    };

    let auth = InstalledFlowAuthenticator::builder(secret, method)
        .persist_tokens_to_disk(&config.token_cache)
        .build()
        .await?;

    Ok(auth)
}
