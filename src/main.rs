//! google-mcp — a standalone stdio MCP server for Gmail + Google Calendar.
//!
//! Usage:
//!   google-mcp        Run the MCP server on stdio (default).
//!   google-mcp auth   Run the one-time OAuth consent flow and cache the token.
//!   google-mcp help   Show this help.
//!
//! Configuration is environment-only: GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET,
//! optional GOOGLE_TOKEN_CACHE and GOOGLE_OAUTH_METHOD. See README.md.

mod auth;
mod config;
mod google;
mod server;

use tracing_subscriber::EnvFilter;

/// CLI command selected from argv.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Run the MCP server on stdio.
    Serve,
    /// Run the OAuth consent flow and cache the token.
    Auth,
    /// Print help.
    Help,
}

fn parse_args<I>(args: I) -> Command
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut iter = args.into_iter();
    let _program = iter.next(); // skip argv[0]
    match iter.next().map(|s| s.as_ref().to_string()) {
        None => Command::Serve,
        Some(s) if s == "auth" => Command::Auth,
        Some(s) if s == "help" || s == "--help" || s == "-h" => Command::Help,
        Some(_) => Command::Help,
    }
}

fn print_help() {
    println!(
        "\
google-mcp {version} — Gmail + Google Calendar as MCP tools (stdio)

USAGE:
    google-mcp [COMMAND]

COMMANDS:
    (none)   Run the MCP server on stdio (for Helix, grok-cli, Claude Desktop…)
    auth     One-time OAuth consent flow; caches the refresh token, then exits
    help     Show this help

ENVIRONMENT:
    GOOGLE_CLIENT_ID        OAuth 2.0 client ID (required)
    GOOGLE_CLIENT_SECRET    OAuth 2.0 client secret (required)
    GOOGLE_TOKEN_CACHE      Token cache path (default: ~/.config/google-mcp/token.json)
    GOOGLE_OAUTH_METHOD     http (default) or interactive

Run `google-mcp auth` once before first use. See README.md for the
Google Cloud Console setup steps.",
        version = env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls needs an explicit crypto provider when multiple backends are
    // present (same fix as Helix's main.rs).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("google-mcp: failed to install rustls ring provider");

    // Logs must go to stderr: stdout is the MCP JSON-RPC wire.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    match parse_args(std::env::args()) {
        Command::Serve => {
            let clients = google::GoogleClients::from_env().await.map_err(|e| {
                eprintln!("google-mcp: {e:#}");
                e
            })?;
            server::GoogleMcpServer::new(clients).serve_stdio().await
        }
        Command::Auth => {
            let config = config::GoogleConfig::from_env()?;
            println!("Opening the Google consent page in your browser…");
            println!("(On a headless machine, SSH with port forwarding or copy the");
            println!(" resulting token file to this host afterwards.)");
            google::GoogleClients::ensure_token(&config).await?;
            println!(
                "OK: authorized and token cached at {}",
                config.token_cache.display()
            );
            Ok(())
        }
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_means_serve() {
        assert_eq!(parse_args(["google-mcp"]), Command::Serve);
    }

    #[test]
    fn auth_subcommand() {
        assert_eq!(parse_args(["google-mcp", "auth"]), Command::Auth);
    }

    #[test]
    fn help_variants() {
        for flag in ["help", "--help", "-h"] {
            assert_eq!(parse_args(["google-mcp", flag]), Command::Help);
        }
    }

    #[test]
    fn unknown_command_shows_help() {
        assert_eq!(parse_args(["google-mcp", "frobnicate"]), Command::Help);
    }
}
