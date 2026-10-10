//! Thin wrappers around the Gmail and Calendar API hubs.
//!
//! Each method returns a pretty-printed JSON string so the MCP tools can hand
//! the result straight back to the model as text content.

use base64::Engine as _;
use chrono::{DateTime, Utc};

use crate::auth::{self, SCOPES};
use crate::config::GoogleConfig;

pub type HttpsConnector =
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>;
/// The client type the google-apis-rs hubs expect: a legacy hyper client over
/// our TLS connector, with the boxed body type from google-apis-common.
pub type HyperClient = google_gmail1::common::Client<HttpsConnector>;

#[derive(Clone)]
/// Authenticated Gmail + Calendar hubs. The hub type parameter is the *connector*;
/// each hub wraps it in its own `common::Client` internally.
pub struct GoogleClients {
    gmail: google_gmail1::Gmail<HttpsConnector>,
    calendar: google_calendar3::CalendarHub<HttpsConnector>,
}

fn hyper_client() -> anyhow::Result<HyperClient> {
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()?
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();
    Ok(hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build(connector))
}

impl GoogleClients {
    /// Build both API hubs from environment configuration.
    ///
    /// Performs no network I/O unless the token cache is missing/expired and a
    /// call is made. (The interactive consent flow only triggers from
    /// `google-mcp auth` or on first real API use.)
    pub async fn from_env() -> anyhow::Result<Self> {
        let config = GoogleConfig::from_env()?;
        let auth = auth::build_authenticator(&config).await?;
        let client = hyper_client()?;
        Ok(Self {
            gmail: google_gmail1::Gmail::new(client.clone(), auth.clone()),
            calendar: google_calendar3::CalendarHub::new(client, auth),
        })
    }

    /// Force a token fetch (runs the OAuth consent flow on first use).
    /// Used by the `auth` subcommand; not by the MCP tools.
    pub async fn ensure_token(config: &GoogleConfig) -> anyhow::Result<()> {
        let auth = auth::build_authenticator(config).await?;
        let _token = auth.token(SCOPES).await?;
        Ok(())
    }

    // ── Gmail ─────────────────────────────────────────────────────────────

    /// Search Gmail. Returns message/thread IDs plus a result-size estimate.
    ///
    /// NOTE: every API call below pins `.clear_scopes().add_scopes(SCOPES)`.
    /// yup-oauth2 caches tokens keyed by the *exact* requested scope set, and
    /// the google-apis-rs builders default to per-method minimal scopes
    /// (e.g. `gmail.readonly` for `messages.list`). A scope-set mismatch
    /// makes yup-oauth2 start a new interactive browser flow, which hangs
    /// forever on a headless server. Pinning our canonical scopes keeps the
    /// token cache hitting.
    pub async fn gmail_search(&self, query: &str, max_results: u32) -> anyhow::Result<String> {
        let max = max_results.clamp(1, 50);
        let (_, list) = self
            .gmail
            .users()
            .messages_list("me")
            .clear_scopes()
            .add_scopes(SCOPES)
            .q(query)
            .max_results(max)
            .doit()
            .await?;
        let messages: Vec<serde_json::Value> = list
            .messages
            .unwrap_or_default()
            .into_iter()
            .map(|m| serde_json::json!({ "id": m.id, "thread_id": m.thread_id }))
            .collect();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "result_size_estimate": list.result_size_estimate,
            "messages": messages,
        }))?)
    }

    /// Read one message: headers, snippet, labels, and the plain-text body.
    pub async fn gmail_read(&self, id: &str) -> anyhow::Result<String> {
        let (_, msg) = self
            .gmail
            .users()
            .messages_get("me", id)
            .clear_scopes()
            .add_scopes(SCOPES)
            .format("full")
            .doit()
            .await?;

        let headers = msg
            .payload
            .as_ref()
            .and_then(|p| p.headers.clone())
            .unwrap_or_default();
        let header = |name: &str| {
            headers
                .iter()
                .find(|h| h.name.as_deref() == Some(name))
                .and_then(|h| h.value.clone())
                .unwrap_or_default()
        };

        let mut body = msg
            .payload
            .as_ref()
            .map(extract_plain_text)
            .unwrap_or_default();
        if body.chars().count() > 8000 {
            body = format!("{}…[truncated]", body.chars().take(8000).collect::<String>());
        }

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": msg.id,
            "thread_id": msg.thread_id,
            "subject": header("Subject"),
            "from": header("From"),
            "to": header("To"),
            "date": header("Date"),
            "labels": msg.label_ids,
            "snippet": msg.snippet,
            "body": body,
        }))?)
    }

    /// Send a plain-text email. In google-gmail1 v7 `messages.send` is only
    /// exposed via the upload path, so the RFC 822 message is uploaded as
    /// `message/rfc822` media (the documented way to send a message).
    pub async fn gmail_send(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<String> {
        let raw = format!(
            "To: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
        );
        let (_, sent) = self
            .gmail
            .users()
            .messages_send(google_gmail1::api::Message::default(), "me")
            .clear_scopes()
            .add_scopes(SCOPES)
            .upload(
                std::io::Cursor::new(raw.into_bytes()),
                "message/rfc822".parse().unwrap(),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": sent.id,
            "thread_id": sent.thread_id,
            "labels": sent.label_ids,
        }))?)
    }

    /// Add/remove label IDs on a message.
    ///
    /// Useful label IDs: `UNREAD` (mark read by removing it), `INBOX` (archive
    /// by removing it), `STARRED`, `TRASH`, `SPAM`.
    pub async fn gmail_modify(
        &self,
        id: &str,
        add_labels: Vec<String>,
        remove_labels: Vec<String>,
    ) -> anyhow::Result<String> {
        let request = google_gmail1::api::ModifyMessageRequest {
            add_label_ids: Some(add_labels),
            remove_label_ids: Some(remove_labels),
        };
        let (_, msg) = self
            .gmail
            .users()
            .messages_modify(request, "me", id)
            .clear_scopes()
            .add_scopes(SCOPES)
            .doit()
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": msg.id,
            "thread_id": msg.thread_id,
            "labels": msg.label_ids,
        }))?)
    }

    // ── Calendar ──────────────────────────────────────────────────────────

    /// List events on the primary calendar in `[time_min, time_max)`.
    /// Times are RFC 3339, e.g. `2026-10-02T00:00:00-04:00`.
    pub async fn calendar_list_events(
        &self,
        time_min: &str,
        time_max: &str,
        max_results: u32,
    ) -> anyhow::Result<String> {
        let time_min = parse_rfc3339(time_min)?;
        let time_max = parse_rfc3339(time_max)?;
        let (_, list) = self
            .calendar
            .events()
            .list("primary")
            .clear_scopes()
            .add_scopes(SCOPES)
            .time_min(time_min)
            .time_max(time_max)
            .max_results(max_results.clamp(1, 100) as i32)
            .single_events(true)
            .order_by("startTime")
            .doit()
            .await?;
        let events: Vec<serde_json::Value> = list
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "summary": e.summary,
                    "description": e.description,
                    "start": e.start.as_ref().map(event_time_str),
                    "end": e.end.as_ref().map(event_time_str),
                    "location": e.location,
                    "status": e.status,
                    "html_link": e.html_link,
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "events": events,
        }))?)
    }

    /// Create an event on the primary calendar. Start/end are RFC 3339.
    pub async fn calendar_create_event(
        &self,
        summary: &str,
        start: &str,
        end: &str,
        description: Option<String>,
        attendees: Vec<String>,
    ) -> anyhow::Result<String> {
        // Validate timestamps early for a clean error message.
        let start_dt = parse_rfc3339(start)?;
        let end_dt = parse_rfc3339(end)?;

        let event = google_calendar3::api::Event {
            summary: Some(summary.to_string()),
            description,
            start: Some(google_calendar3::api::EventDateTime {
                date_time: Some(start_dt),
                ..Default::default()
            }),
            end: Some(google_calendar3::api::EventDateTime {
                date_time: Some(end_dt),
                ..Default::default()
            }),
            attendees: Some(
                attendees
                    .into_iter()
                    .map(|email| google_calendar3::api::EventAttendee {
                        email: Some(email),
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let (_, created) = self.calendar.events().insert(event, "primary").clear_scopes().add_scopes(SCOPES).doit().await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": created.id,
            "summary": created.summary,
            "html_link": created.html_link,
            "start": created.start.as_ref().map(event_time_str),
            "end": created.end.as_ref().map(event_time_str),
        }))?)
    }
}

/// Render an `EventDateTime` as a string: RFC 3339 for timed events,
/// `YYYY-MM-DD` for all-day events.
fn event_time_str(s: &google_calendar3::api::EventDateTime) -> String {
    if let Some(dt) = s.date_time {
        dt.to_rfc3339()
    } else if let Some(d) = s.date {
        d.to_string()
    } else {
        String::new()
    }
}

fn parse_rfc3339(s: &str) -> anyhow::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow::anyhow!("'{s}' is not a valid RFC 3339 timestamp: {e}"))
}

/// Recursively find the first `text/plain` part and decode its base64url body.
fn extract_plain_text(part: &google_gmail1::api::MessagePart) -> String {
    if part.mime_type.as_deref() == Some("text/plain") {
        if let Some(data) = part.body.as_ref().and_then(|b| b.data.clone()) {
            if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&data) {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }
    if let Some(parts) = part.parts.as_ref() {
        for p in parts {
            let text = extract_plain_text(p);
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}
