# google-mcp

A standalone [Model Context Protocol](https://modelcontextprotocol.io/) server that
exposes **Gmail** and **Google Calendar** as tools over **stdio**. Written in Rust,
built for John's agent stack: it plugs into [Helix](https://github.com/micro-tech/bot)
today and [grok-cli](https://github.com/micro-tech/grok-cli) next, via a generic MCP
client in each.

No secrets live in this repo. Auth is OAuth 2.0 (installed-app flow) configured
**only** from environment variables; the first run caches a refresh token on disk.

## Tools

| Tool | What it does |
|---|---|
| `gmail_search` | Search Gmail (`query`, `max_results`). Returns message/thread IDs. |
| `gmail_read` | Read one message (`id`) → subject, from, to, date, labels, snippet, plain-text body. |
| `gmail_send` | Send a plain-text email (`to`, `subject`, `body`). |
| `gmail_modify` | Add/remove label IDs (`id`, `add_labels`, `remove_labels`). Remove `UNREAD` to mark read, `INBOX` to archive, add `TRASH` to delete. |
| `calendar_list_events` | List primary-calendar events in `[time_min, time_max)` (RFC 3339), start-time order. |
| `calendar_create_event` | Create an event (`summary`, `start`, `end`, optional `description`, `attendees`). |

## 1. Google Cloud Console setup (one time)

You need an **OAuth 2.0 Client ID** of type **Desktop app**. Steps:

1. Go to the [Google Cloud Console](https://console.cloud.google.com/) and create
   (or select) a project.
2. **Enable the APIs**: left menu → *APIs & Services → Library*. Search for and
   enable **Gmail API** and **Google Calendar API**.
3. **Configure the consent screen**: *APIs & Services → OAuth consent screen*.
   - User type: **External**.
   - Fill in app name, user support email, developer contact email.
   - On the *Scopes* page you can skip adding scopes (the app requests
     `gmail.modify` and `calendar` at runtime).
   - **Important while testing**: on the *Audience* page, under *Test users*,
     add the Google account(s) that will use this. Until the app is published,
     only test users can authorize it. (Publishing/verification is only needed
     if other people will use it.)
4. **Create the client ID**: *APIs & Services → Credentials → Create Credentials
   → OAuth client ID*.
   - Application type: **Desktop app**.
   - Name it e.g. `google-mcp`.
   - Google shows you a **Client ID** and **Client secret** — copy both.
5. Export them in your shell (or your service's environment):

```sh
export GOOGLE_CLIENT_ID="....apps.googleusercontent.com"
export GOOGLE_CLIENT_SECRET="GOCSPX-..."
# optional:
export GOOGLE_TOKEN_CACHE="$HOME/.config/google-mcp/token.json"  # this is the default
```

## 2. Build and authorize

```sh
cargo build --release
# one-time consent flow (opens the browser; caches the refresh token):
./target/release/google-mcp auth
```

`google-mcp auth` spins up a localhost redirect server and prints/opens the
Google consent URL. Approve it and the refresh token is written to
`$GOOGLE_TOKEN_CACHE`. The MCP server itself never opens a browser.

> **Headless server (e.g. the R630):** run `google-mcp auth` on any machine with
> a browser (your laptop works), then copy the resulting `token.json` to
> `~/.config/google-mcp/token.json` on the server (or set `GOOGLE_TOKEN_CACHE`
> to wherever you put it). Alternatively SSH in with port forwarding so the
> localhost redirect reaches your browser.

## 3. Run as an MCP server

The server speaks MCP over stdio — just spawn it:

```sh
./target/release/google-mcp
```

It reads JSON-RPC on stdin, writes responses on stdout, and logs to **stderr**
(never stdout). It exits when the client closes stdin.

### Wiring it into a client

Any MCP client that can spawn a stdio server works. Example config JSON
(the shape Claude Desktop, Helix, and grok-cli-style clients all use):

```json
{
  "mcpServers": {
    "google": {
      "command": "/home/john/bin/google-mcp",
      "args": [],
      "env": {
        "GOOGLE_CLIENT_ID": "....apps.googleusercontent.com",
        "GOOGLE_CLIENT_SECRET": "GOCSPX-...",
        "GOOGLE_TOKEN_CACHE": "/home/john/.config/google-mcp/token.json"
      }
    }
  }
}
```

For Helix/grok-cli (Rust, `rmcp` client): spawn the binary as a child process
(`rmcp`'s `transport-child-process` feature), run the MCP `initialize`
handshake, then `tools/list` will show the six tools above and `tools/call`
invokes them. Tool arguments are plain JSON matching the schemas in
`src/server.rs`.

### Quick smoke test without a full client

```sh
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
 | GOOGLE_CLIENT_ID=... GOOGLE_CLIENT_SECRET=... ./target/release/google-mcp \
 | head -c 2000
```

(Requires a cached token from `google-mcp auth` first — the server needs valid
config to start, though `tools/list` itself makes no Google calls.)

## Development

```sh
cargo build          # debug build
cargo test           # unit tests (no network, no credentials needed)
cargo test -- --ignored   # includes the live-API smoke test (needs real creds + cached token)
```

## Notes

- Scopes requested: `https://www.googleapis.com/auth/gmail.modify` (read, send,
  and label management — but **not** permanent delete) and
  `https://www.googleapis.com/auth/calendar` (read/write the primary calendar).
- Token refresh is handled automatically by `yup-oauth2`; the cache file is
  rewritten when tokens rotate. Treat it like a password.
- If Google returns `403 insufficientPermissions`, the consent screen grant
  didn't include a scope — re-run `google-mcp auth` (delete the old
  `token.json` first) and make sure you tick both scopes.
