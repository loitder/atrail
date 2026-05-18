use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SOURCE_CODEX_JSONL: &str = "codex_jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyMode {
    None,
    Safe,
    Strict,
}

impl PrivacyMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "safe" => Some(Self::Safe),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ImportBatch {
    pub sessions: BTreeMap<String, Session>,
    pub turns: BTreeMap<String, Turn>,
    pub spans: BTreeMap<String, Span>,
    pub events: Vec<Event>,
    pub token_usage: Vec<TokenUsage>,
    pub tool_calls: BTreeMap<String, ToolCall>,
    pub artifacts: Vec<Artifact>,
    pub unknown_events: Vec<UnknownEvent>,
}

impl ImportBatch {
    pub fn merge(&mut self, other: ImportBatch) {
        self.sessions.extend(other.sessions);
        self.turns.extend(other.turns);
        self.spans.extend(other.spans);
        self.events.extend(other.events);
        self.token_usage.extend(other.token_usage);
        self.tool_calls.extend(other.tool_calls);
        self.artifacts.extend(other.artifacts);
        self.unknown_events.extend(other.unknown_events);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub source: String,
    pub project_id: Option<String>,
    pub project_path_hash: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub model: Option<String>,
    pub codex_version: Option<String>,
    pub auth_mode: Option<String>,
    pub originator: Option<String>,
    pub raw_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub session_id: String,
    pub turn_index: i64,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub user_message_hash: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub tool_call_count: i64,
    pub error_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub span_type: String,
    pub name: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub span_id: Option<String>,
    pub event_type: String,
    pub timestamp: String,
    pub attributes: Value,
    pub raw_event: Value,
    pub raw_source: String,
    pub raw_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub span_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<f64>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub span_id: String,
    pub tool_type: Option<String>,
    pub tool_name: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub input_hash: Option<String>,
    pub output_hash: Option<String>,
    pub error_type: Option<String>,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub artifact_type: String,
    pub path_hash: Option<String>,
    pub path_display: Option<String>,
    pub operation: String,
    pub size_bytes: Option<i64>,
    pub attributes: Value,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownEvent {
    pub id: String,
    pub raw_event: Value,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct InspectReport {
    pub files: usize,
    pub sessions: usize,
    pub events: usize,
    pub unknown_events: usize,
    pub event_types: BTreeMap<String, usize>,
    pub payload_types: BTreeMap<String, usize>,
}

impl InspectReport {
    pub fn add_event(&mut self, event_type: &str, payload_type: Option<&str>) {
        self.events += 1;
        *self.event_types.entry(event_type.to_string()).or_default() += 1;
        if let Some(payload_type) = payload_type {
            *self
                .payload_types
                .entry(payload_type.to_string())
                .or_default() += 1;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub source: String,
    pub project_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: Option<f64>,
    pub input_cost_usd: Option<f64>,
    pub cached_input_cost_usd: Option<f64>,
    pub output_cost_usd: Option<f64>,
    pub cost_model: Option<String>,
    pub cost_source: Option<String>,
    pub tool_call_count: i64,
    pub error_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineItem {
    pub timestamp: String,
    pub event_type: String,
    pub turn_id: Option<String>,
    pub span_id: Option<String>,
    pub status: Option<String>,
    pub name: Option<String>,
    pub duration_ms: Option<i64>,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSummary {
    pub sessions: i64,
    pub turns: i64,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub tool_calls: i64,
    pub failed_tool_calls: i64,
    pub errors: i64,
    pub avg_session_duration_ms: Option<f64>,
    pub avg_turn_duration_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct TokenUsageByModel {
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
}
