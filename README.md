# atrail

atrail is a local-first observability tool for Codex session history. It reads
Codex JSONL session files, normalizes them into sessions, turns, events, spans,
token usage, tool calls, and artifacts, stores the result in SQLite, and serves
a small dashboard for exploring agent behavior.

The project is currently focused on Codex CLI session data under
`~/.codex/sessions`.

## What It Shows

- Session list with model, status, duration, tokens, tool calls, and errors.
- Per-session timeline with turns, model messages, tool calls, shell commands,
  patch events, web search events, token usage, and errors.
- Aggregate metrics for sessions, turns, token mix, average turn latency, tool
  calls, failed tools, and errors.
- Local JSON API for scripts or custom dashboards.

## Quick Start

```bash
cargo run -- doctor
cargo run -- inspect ~/.codex/sessions
cargo run -- ingest ~/.codex/sessions
cargo run -- serve
```

Then open:

```text
http://127.0.0.1:4319/
```

By default, atrail reads from:

```text
~/.codex/sessions
```

and writes its SQLite database to:

```text
~/.atrail/atrail.db
```

## Installation

Build a release binary:

```bash
cargo build --release
```

Run it directly:

```bash
./target/release/atrail doctor
./target/release/atrail ingest ~/.codex/sessions
./target/release/atrail serve
```

## CLI

```text
atrail inspect [SOURCE]
atrail ingest [SOURCE] [--db PATH] [--privacy safe|strict|none]
atrail sessions [--db PATH] [--limit N]
atrail timeline SESSION_ID [--db PATH]
atrail summary [--db PATH]
atrail serve [--db PATH] [--host 127.0.0.1] [--port 4319]
atrail doctor
```

### Inspect Session Files

`inspect` scans JSONL files and reports how many files, sessions, events,
payload types, and unknown events atrail can see.

```bash
cargo run -- inspect ~/.codex/sessions
```

### Ingest Into SQLite

`ingest` parses Codex sessions and upserts normalized records into SQLite.

```bash
cargo run -- ingest ~/.codex/sessions --db ~/.atrail/atrail.db
```

The default privacy mode is `safe`:

```bash
cargo run -- ingest ~/.codex/sessions --privacy strict
```

### Query Data

```bash
cargo run -- sessions --limit 20
cargo run -- summary
cargo run -- timeline SESSION_ID
```

### Serve The Dashboard

```bash
cargo run -- serve --host 127.0.0.1 --port 4319
```

The dashboard is served from the same process as the JSON API.

## JSON API

When `atrail serve` is running:

```text
GET /api
GET /api/sessions
GET /api/sessions/{id}
GET /api/sessions/{id}/timeline
GET /api/metrics/summary
GET /api/pricing/openai
GET /api/admin/status
```

Example:

```bash
curl http://127.0.0.1:4319/api/metrics/summary
```

## Privacy Modes

atrail is designed for local analysis, and the default mode avoids putting
prompt, response, command, and path text into normalized attributes.

```text
safe    Hashes sensitive text and keeps small hints such as command names or
        path basenames.
strict  Stores presence, counts, hashes, and command names where available, but
        hides path display values.
none    Stores full text/path/command values in normalized attributes.
```

Important: the current SQLite schema also stores `raw_event_json` for debugging
and replay. Treat the database as sensitive local data and do not share it unless
you are comfortable sharing the original Codex session contents.

## Data Model

atrail normalizes Codex JSONL into:

- `sessions`: one Codex thread/session.
- `turns`: one user-to-agent interaction cycle.
- `events`: timestamped normalized behavior events.
- `spans`: trace-like spans for tool calls and related work.
- `token_usage`: input, output, cached, reasoning, and total token counts.
- `tool_calls`: shell, patch, MCP, web, and function/custom tool calls.
- `artifacts`: file/path-oriented outputs where detected.
- `unknown_events`: raw events that are not recognized yet.

## Project Layout

```text
src/main.rs          CLI entry point
src/codex.rs         Codex JSONL inspection and ingestion
src/model.rs         Core data structures
src/storage.rs       SQLite schema and queries
src/api.rs           Minimal HTTP server and JSON API
src/ui/              Embedded dashboard assets
src/redact.rs        Privacy helpers and hashing
tests/               Parser/storage integration tests
docs/                Product and design notes
```

## Development

Run the test suite:

```bash
cargo test
```

Try the parser against the fixture data:

```bash
cargo run -- inspect tests/fixtures
cargo run -- ingest tests/fixtures --db /tmp/atrail-fixture.db
cargo run -- sessions --db /tmp/atrail-fixture.db
```

Format before committing:

```bash
cargo fmt
```

## Current Scope

atrail is an early local-first implementation. It does not proxy Codex requests
and does not upload data. Cost display is an API-price estimate, not a Codex or
ChatGPT subscription bill: the API tries to read OpenAI's public pricing page at
runtime and falls back to a bundled price table when the page cannot be fetched.
OpenTelemetry ingestion and support for additional coding agents are future
extensions.
