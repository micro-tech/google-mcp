//! The MCP server: six Gmail/Calendar tools over stdio.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::io::stdio,
};

use crate::google::GoogleClients;

// ── Tool parameter schemas ────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GmailSearchParams {
    /// Gmail search query, e.g. "from:boss@example.com subject:invoice newer_than:7d".
    /// See https://support.google.com/mail/answer/7190 for the full syntax.
    pub query: String,
    /// Max messages to return (1-50, default 10).
    #[serde(default = "default_search_limit")]
    pub max_results: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GmailReadParams {
    /// The Gmail message ID (from gmail_search).
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GmailSendParams {
    /// Recipient address (comma-separated for several).
    pub to: String,
    /// Subject line.
    pub subject: String,
    /// Plain-text body.
    pub body: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GmailModifyParams {
    /// The Gmail message ID.
    pub id: String,
    /// Label IDs to add, e.g. ["STARRED"].
    #[serde(default)]
    pub add_labels: Vec<String>,
    /// Label IDs to remove, e.g. ["UNREAD"] to mark read, ["INBOX"] to archive.
    #[serde(default)]
    pub remove_labels: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CalendarListParams {
    /// Lower bound (inclusive), RFC 3339, e.g. "2026-10-02T00:00:00-04:00".
    pub time_min: String,
    /// Upper bound (exclusive), RFC 3339.
    pub time_max: String,
    /// Max events to return (1-100, default 25).
    #[serde(default = "default_event_limit")]
    pub max_results: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CalendarCreateParams {
    /// Event title.
    pub summary: String,
    /// Start time, RFC 3339 with timezone offset, e.g. "2026-10-05T14:00:00-04:00".
    pub start: String,
    /// End time, RFC 3339 with timezone offset.
    pub end: String,
    /// Optional longer description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional attendee email addresses.
    #[serde(default)]
    pub attendees: Vec<String>,
}

fn default_search_limit() -> u32 {
    10
}

fn default_event_limit() -> u32 {
    25
}

// ── Server ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GoogleMcpServer {
    tool_router: ToolRouter<Self>,
    clients: Arc<GoogleClients>,
}

impl GoogleMcpServer {
    pub fn new(clients: GoogleClients) -> Self {
        Self {
            tool_router: Self::tool_router(),
            clients: Arc::new(clients),
        }
    }

    /// Serve this server on stdio until the client disconnects.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let running = self.serve(stdio()).await?;
        tracing::info!("google-mcp serving on stdio");
        running.waiting().await?;
        Ok(())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GoogleMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("google-mcp", env!("CARGO_PKG_VERSION")).with_description(
                    "Gmail and Google Calendar tools for a personal assistant.",
                ),
            )
            .with_instructions(
                "Use gmail_search to find messages, gmail_read for full content, \
                 gmail_send to send mail, gmail_modify to archive/mark-read/star, \
                 calendar_list_events to see what's scheduled, calendar_create_event \
                 to add events. Times are RFC 3339 with timezone offset.",
            )
    }
}

#[tool_router(router = tool_router)]
impl GoogleMcpServer {
    #[tool(
        name = "gmail_search",
        description = "Search Gmail with Gmail search syntax. Returns message IDs and thread IDs; use gmail_read for content."
    )]
    async fn gmail_search(
        &self,
        params: Parameters<GmailSearchParams>,
    ) -> Result<String, McpError> {
        let p = params.0;
        self.clients
            .gmail_search(&p.query, p.max_results)
            .await
            .map_err(to_mcp_error)
    }

    #[tool(
        name = "gmail_read",
        description = "Read a Gmail message by ID. Returns subject, from, to, date, labels, snippet, and plain-text body."
    )]
    async fn gmail_read(&self, params: Parameters<GmailReadParams>) -> Result<String, McpError> {
        self.clients
            .gmail_read(&params.0.id)
            .await
            .map_err(to_mcp_error)
    }

    #[tool(
        name = "gmail_send",
        description = "Send a plain-text email via Gmail."
    )]
    async fn gmail_send(&self, params: Parameters<GmailSendParams>) -> Result<String, McpError> {
        let p = params.0;
        self.clients
            .gmail_send(&p.to, &p.subject, &p.body)
            .await
            .map_err(to_mcp_error)
    }

    #[tool(
        name = "gmail_modify",
        description = "Add or remove Gmail label IDs on a message. Remove UNREAD to mark read, remove INBOX to archive, add STARRED to star, add TRASH to delete."
    )]
    async fn gmail_modify(
        &self,
        params: Parameters<GmailModifyParams>,
    ) -> Result<String, McpError> {
        let p = params.0;
        self.clients
            .gmail_modify(&p.id, p.add_labels, p.remove_labels)
            .await
            .map_err(to_mcp_error)
    }

    #[tool(
        name = "calendar_list_events",
        description = "List events on the primary Google Calendar between two RFC 3339 timestamps, in start-time order."
    )]
    async fn calendar_list_events(
        &self,
        params: Parameters<CalendarListParams>,
    ) -> Result<String, McpError> {
        let p = params.0;
        self.clients
            .calendar_list_events(&p.time_min, &p.time_max, p.max_results)
            .await
            .map_err(to_mcp_error)
    }

    #[tool(
        name = "calendar_create_event",
        description = "Create an event on the primary Google Calendar. Start/end are RFC 3339 with timezone offset."
    )]
    async fn calendar_create_event(
        &self,
        params: Parameters<CalendarCreateParams>,
    ) -> Result<String, McpError> {
        let p = params.0;
        self.clients
            .calendar_create_event(&p.summary, &p.start, &p.end, p.description, p.attendees)
            .await
            .map_err(to_mcp_error)
    }
}

fn to_mcp_error(e: anyhow::Error) -> McpError {
    // Report API failures as tool errors (is_error=true) rather than killing
    // the MCP session with a protocol error.
    McpError::internal_error(e.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tool router is generated without needing Google credentials, so
    /// schema tests run offline.
    fn tool_names() -> Vec<String> {
        GoogleMcpServer::tool_router()
            .list_all()
            .iter()
            .map(|t| t.name.clone().into_owned())
            .collect()
    }

    #[test]
    fn exposes_all_six_tools() {
        let mut names = tool_names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "calendar_create_event",
                "calendar_list_events",
                "gmail_modify",
                "gmail_read",
                "gmail_search",
                "gmail_send",
            ]
        );
    }

    #[test]
    fn tool_schemas_have_expected_properties() {
        let router = GoogleMcpServer::tool_router();
        let tools: std::collections::HashMap<_, _> = router
            .list_all()
            .iter()
            .map(|t| (t.name.clone().into_owned(), t.clone()))
            .collect();

        let search = &tools["gmail_search"];
        let schema = serde_json::to_value(&search.input_schema).unwrap();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
        assert!(schema["required"].as_array().unwrap().contains(&serde_json::json!("query")));

        let create = &tools["calendar_create_event"];
        let schema = serde_json::to_value(&create.input_schema).unwrap();
        let required = schema["required"].as_array().unwrap();
        for field in ["summary", "start", "end"] {
            assert!(required.contains(&serde_json::json!(field)), "missing {field}");
        }

        let modify = &tools["gmail_modify"];
        let schema = serde_json::to_value(&modify.input_schema).unwrap();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("add_labels"));
        assert!(props.contains_key("remove_labels"));
    }

    #[test]
    fn tool_descriptions_are_nonempty() {
        for tool in GoogleMcpServer::tool_router().list_all() {
            let desc = tool.description.as_deref().unwrap_or("");
            assert!(!desc.is_empty(), "tool {} has no description", tool.name);
        }
    }

    /// Hits the real Gmail API — needs GOOGLE_CLIENT_ID/SECRET plus a cached
    /// token, so it never runs in CI. Run manually with:
    /// `cargo test -- --ignored`
    #[ignore]
    #[tokio::test]
    async fn live_gmail_search_smoke() {
        let clients = GoogleClients::from_env()
            .await
            .expect("needs GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET and a cached token");
        let out = clients
            .gmail_search("in:inbox", 1)
            .await
            .expect("gmail_search failed");
        assert!(out.contains("messages"));
    }
}
