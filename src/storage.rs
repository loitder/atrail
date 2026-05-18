use crate::model::*;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create db dir {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    pub fn insert_batch(&mut self, batch: &ImportBatch) -> Result<()> {
        let tx = self.conn.transaction()?;
        for session in batch.sessions.values() {
            tx.execute(
                "INSERT INTO sessions
                 (id, source, project_id, project_path_hash, started_at, ended_at, duration_ms,
                  status, model, codex_version, auth_mode, originator, raw_file_path, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, CURRENT_TIMESTAMP)
                 ON CONFLICT(id) DO UPDATE SET
                   source=excluded.source,
                   project_id=excluded.project_id,
                   project_path_hash=excluded.project_path_hash,
                   started_at=excluded.started_at,
                   ended_at=excluded.ended_at,
                   duration_ms=excluded.duration_ms,
                   status=excluded.status,
                   model=excluded.model,
                   codex_version=excluded.codex_version,
                   auth_mode=excluded.auth_mode,
                   originator=excluded.originator,
                   raw_file_path=excluded.raw_file_path,
                   updated_at=CURRENT_TIMESTAMP",
                params![
                    session.id,
                    session.source,
                    session.project_id,
                    session.project_path_hash,
                    session.started_at,
                    session.ended_at,
                    session.duration_ms,
                    session.status,
                    session.model,
                    session.codex_version,
                    session.auth_mode,
                    session.originator,
                    session.raw_file_path
                ],
            )?;
        }

        for turn in batch.turns.values() {
            tx.execute(
                "INSERT INTO turns
                 (id, session_id, turn_index, started_at, ended_at, duration_ms, status,
                  user_message_hash, input_tokens, output_tokens, cached_tokens, reasoning_tokens,
                  tool_call_count, error_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id=excluded.session_id,
                   turn_index=excluded.turn_index,
                   started_at=excluded.started_at,
                   ended_at=excluded.ended_at,
                   duration_ms=excluded.duration_ms,
                   status=excluded.status,
                   user_message_hash=excluded.user_message_hash,
                   input_tokens=excluded.input_tokens,
                   output_tokens=excluded.output_tokens,
                   cached_tokens=excluded.cached_tokens,
                   reasoning_tokens=excluded.reasoning_tokens,
                   tool_call_count=excluded.tool_call_count,
                   error_count=excluded.error_count",
                params![
                    turn.id,
                    turn.session_id,
                    turn.turn_index,
                    turn.started_at,
                    turn.ended_at,
                    turn.duration_ms,
                    turn.status,
                    turn.user_message_hash,
                    turn.input_tokens,
                    turn.output_tokens,
                    turn.cached_tokens,
                    turn.reasoning_tokens,
                    turn.tool_call_count,
                    turn.error_count,
                ],
            )?;
        }

        for span in batch.spans.values() {
            tx.execute(
                "INSERT INTO spans
                 (id, trace_id, parent_span_id, session_id, turn_id, span_type, name, started_at,
                  ended_at, duration_ms, status, attributes_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                   trace_id=excluded.trace_id,
                   parent_span_id=excluded.parent_span_id,
                   session_id=excluded.session_id,
                   turn_id=excluded.turn_id,
                   span_type=excluded.span_type,
                   name=excluded.name,
                   started_at=excluded.started_at,
                   ended_at=excluded.ended_at,
                   duration_ms=excluded.duration_ms,
                   status=excluded.status,
                   attributes_json=excluded.attributes_json",
                params![
                    span.id,
                    span.trace_id,
                    span.parent_span_id,
                    span.session_id,
                    span.turn_id,
                    span.span_type,
                    span.name,
                    span.started_at,
                    span.ended_at,
                    span.duration_ms,
                    span.status,
                    to_json(&span.attributes)?,
                ],
            )?;
        }

        for event in &batch.events {
            tx.execute(
                "INSERT OR IGNORE INTO events
                 (id, session_id, turn_id, span_id, event_type, timestamp, attributes_json,
                  raw_event_json, raw_source, raw_ref)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    event.id,
                    event.session_id,
                    event.turn_id,
                    event.span_id,
                    event.event_type,
                    event.timestamp,
                    to_json(&event.attributes)?,
                    to_json(&event.raw_event)?,
                    event.raw_source,
                    event.raw_ref,
                ],
            )?;
        }

        for usage in &batch.token_usage {
            tx.execute(
                "INSERT INTO token_usage
                 (id, session_id, turn_id, span_id, model, input_tokens, output_tokens,
                  cached_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id=excluded.session_id,
                   turn_id=excluded.turn_id,
                   span_id=excluded.span_id,
                   model=excluded.model,
                   input_tokens=excluded.input_tokens,
                   output_tokens=excluded.output_tokens,
                   cached_tokens=excluded.cached_tokens,
                   reasoning_tokens=excluded.reasoning_tokens,
                   total_tokens=excluded.total_tokens,
                   estimated_cost_usd=excluded.estimated_cost_usd,
                   timestamp=excluded.timestamp",
                params![
                    usage.id,
                    usage.session_id,
                    usage.turn_id,
                    usage.span_id,
                    usage.model,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cached_tokens,
                    usage.reasoning_tokens,
                    usage.total_tokens,
                    usage.estimated_cost_usd,
                    usage.timestamp,
                ],
            )?;
        }

        for tool in batch.tool_calls.values() {
            tx.execute(
                "INSERT INTO tool_calls
                 (id, session_id, turn_id, span_id, tool_type, tool_name, started_at, ended_at,
                  duration_ms, status, exit_code, input_hash, output_hash, error_type,
                  attributes_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id=excluded.session_id,
                   turn_id=excluded.turn_id,
                   span_id=excluded.span_id,
                   tool_type=excluded.tool_type,
                   tool_name=excluded.tool_name,
                   started_at=excluded.started_at,
                   ended_at=excluded.ended_at,
                   duration_ms=excluded.duration_ms,
                   status=excluded.status,
                   exit_code=excluded.exit_code,
                   input_hash=excluded.input_hash,
                   output_hash=excluded.output_hash,
                   error_type=excluded.error_type,
                   attributes_json=excluded.attributes_json",
                params![
                    tool.id,
                    tool.session_id,
                    tool.turn_id,
                    tool.span_id,
                    tool.tool_type,
                    tool.tool_name,
                    tool.started_at,
                    tool.ended_at,
                    tool.duration_ms,
                    tool.status,
                    tool.exit_code,
                    tool.input_hash,
                    tool.output_hash,
                    tool.error_type,
                    to_json(&tool.attributes)?,
                ],
            )?;
        }

        for artifact in &batch.artifacts {
            tx.execute(
                "INSERT OR REPLACE INTO artifacts
                 (id, session_id, turn_id, artifact_type, path_hash, path_display, operation,
                  size_bytes, attributes_json, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    artifact.id,
                    artifact.session_id,
                    artifact.turn_id,
                    artifact.artifact_type,
                    artifact.path_hash,
                    artifact.path_display,
                    artifact.operation,
                    artifact.size_bytes,
                    to_json(&artifact.attributes)?,
                    artifact.timestamp,
                ],
            )?;
        }

        for unknown in &batch.unknown_events {
            tx.execute(
                "INSERT OR IGNORE INTO unknown_events (id, raw_event_json, reason)
                 VALUES (?1, ?2, ?3)",
                params![unknown.id, to_json(&unknown.raw_event)?, unknown.reason],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn sessions(&self, limit: i64) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT
               s.id, s.source, s.project_id, s.started_at, s.ended_at, s.duration_ms,
               s.status, s.model,
               COALESCE(SUM(t.input_tokens), 0) AS input_tokens,
               COALESCE(SUM(t.output_tokens), 0) AS output_tokens,
               COALESCE(SUM(t.cached_tokens), 0) AS cached_tokens,
               COALESCE(SUM(t.reasoning_tokens), 0) AS reasoning_tokens,
               COALESCE(SUM(t.total_tokens), 0) AS total_tokens,
               (SELECT COUNT(*) FROM tool_calls tc WHERE tc.session_id = s.id) AS tool_count,
               (SELECT COUNT(*) FROM events e WHERE e.session_id = s.id AND e.event_type = 'error')
               + (SELECT COUNT(*) FROM tool_calls tc WHERE tc.session_id = s.id AND tc.status = 'failed')
             FROM sessions s
             LEFT JOIN token_usage t ON t.session_id = s.id
             GROUP BY s.id
             ORDER BY s.started_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                source: row.get(1)?,
                project_id: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                duration_ms: row.get(5)?,
                status: row.get(6)?,
                model: row.get(7)?,
                input_tokens: row.get(8)?,
                output_tokens: row.get(9)?,
                cached_tokens: row.get(10)?,
                reasoning_tokens: row.get(11)?,
                total_tokens: row.get(12)?,
                estimated_cost_usd: None,
                input_cost_usd: None,
                cached_input_cost_usd: None,
                output_cost_usd: None,
                cost_model: None,
                cost_source: None,
                tool_call_count: row.get(13)?,
                error_count: row.get(14)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionSummary>> {
        let mut rows = self.sessions(10_000)?;
        Ok(rows.drain(..).find(|session| session.id == id))
    }

    pub fn timeline(&self, session_id: &str) -> Result<Vec<TimelineItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.timestamp, e.event_type, e.turn_id, e.span_id, s.status, s.name,
                    s.duration_ms, e.attributes_json
             FROM events e
             LEFT JOIN spans s ON s.id = e.span_id
             WHERE e.session_id = ?1
             ORDER BY e.timestamp ASC, e.raw_ref ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let attrs: String = row.get(7)?;
            Ok(TimelineItem {
                timestamp: row.get(0)?,
                event_type: row.get(1)?,
                turn_id: row.get(2)?,
                span_id: row.get(3)?,
                status: row.get(4)?,
                name: row.get(5)?,
                duration_ms: row.get(6)?,
                attributes: serde_json::from_str(&attrs).unwrap_or(Value::Null),
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn metrics_summary(&self) -> Result<MetricsSummary> {
        let mut stmt = self.conn.prepare(
            "SELECT
               (SELECT COUNT(*) FROM sessions),
               (SELECT COUNT(*) FROM turns),
               COALESCE((SELECT SUM(total_tokens) FROM token_usage), 0),
               COALESCE((SELECT SUM(input_tokens) FROM token_usage), 0),
               COALESCE((SELECT SUM(output_tokens) FROM token_usage), 0),
               COALESCE((SELECT SUM(cached_tokens) FROM token_usage), 0),
               COALESCE((SELECT SUM(reasoning_tokens) FROM token_usage), 0),
               (SELECT COUNT(*) FROM tool_calls),
               (SELECT COUNT(*) FROM tool_calls WHERE status = 'failed'),
               (SELECT COUNT(*) FROM events WHERE event_type = 'error'),
               (SELECT AVG(duration_ms) FROM sessions WHERE duration_ms IS NOT NULL),
               (SELECT AVG(duration_ms) FROM turns WHERE duration_ms IS NOT NULL)",
        )?;
        stmt.query_row([], |row| {
            Ok(MetricsSummary {
                sessions: row.get(0)?,
                turns: row.get(1)?,
                total_tokens: row.get(2)?,
                input_tokens: row.get(3)?,
                output_tokens: row.get(4)?,
                cached_tokens: row.get(5)?,
                reasoning_tokens: row.get(6)?,
                tool_calls: row.get(7)?,
                failed_tool_calls: row.get(8)?,
                errors: row.get(9)?,
                avg_session_duration_ms: row.get(10)?,
                avg_turn_duration_ms: row.get(11)?,
            })
        })
        .optional()?
        .context("summary query returned no row")
    }

    pub fn token_usage_by_model(&self) -> Result<Vec<TokenUsageByModel>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(t.model, s.model) AS model,
                    COALESCE(SUM(t.input_tokens), 0),
                    COALESCE(SUM(t.output_tokens), 0),
                    COALESCE(SUM(t.cached_tokens), 0),
                    COALESCE(SUM(t.reasoning_tokens), 0)
             FROM token_usage t
             LEFT JOIN sessions s ON s.id = t.session_id
             GROUP BY COALESCE(t.model, s.model)",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TokenUsageByModel {
                model: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cached_tokens: row.get(3)?,
                reasoning_tokens: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }
}

fn to_json(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  project_id TEXT,
  project_path_hash TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  model TEXT,
  codex_version TEXT,
  auth_mode TEXT,
  originator TEXT,
  raw_file_path TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS turns (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_index INTEGER,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  user_message_hash TEXT,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cached_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  tool_call_count INTEGER DEFAULT 0,
  error_count INTEGER DEFAULT 0,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS spans (
  id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  parent_span_id TEXT,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_type TEXT NOT NULL,
  name TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  attributes_json TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id),
  FOREIGN KEY(turn_id) REFERENCES turns(id)
);

CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  event_type TEXT NOT NULL,
  timestamp DATETIME NOT NULL,
  attributes_json TEXT,
  raw_event_json TEXT,
  raw_source TEXT,
  raw_ref TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS token_usage (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  model TEXT,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cached_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  total_tokens INTEGER DEFAULT 0,
  estimated_cost_usd REAL,
  timestamp DATETIME,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS tool_calls (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  span_id TEXT,
  tool_type TEXT,
  tool_name TEXT,
  started_at DATETIME,
  ended_at DATETIME,
  duration_ms INTEGER,
  status TEXT,
  exit_code INTEGER,
  input_hash TEXT,
  output_hash TEXT,
  error_type TEXT,
  attributes_json TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS artifacts (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  turn_id TEXT,
  artifact_type TEXT,
  path_hash TEXT,
  path_display TEXT,
  operation TEXT,
  size_bytes INTEGER,
  attributes_json TEXT,
  timestamp DATETIME,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS unknown_events (
  id TEXT PRIMARY KEY,
  raw_event_json TEXT,
  reason TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at);
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, turn_index);
CREATE INDEX IF NOT EXISTS idx_spans_session ON spans(session_id, started_at);
CREATE INDEX IF NOT EXISTS idx_spans_trace ON spans(trace_id, parent_span_id);
CREATE INDEX IF NOT EXISTS idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id, started_at);
CREATE INDEX IF NOT EXISTS idx_token_usage_session ON token_usage(session_id, timestamp);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
"#;
