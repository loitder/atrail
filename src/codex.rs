use crate::model::*;
use crate::redact::{
    command_attrs, hash_tag, maybe_path_display, merge_json, sha256_hex, short_project_id,
    text_attrs,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct RawLine {
    path: PathBuf,
    line_no: usize,
    value: Value,
    top_type: String,
    payload_type: Option<String>,
    timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct PendingTool {
    call_id: String,
    id: String,
    session_id: String,
    turn_id: Option<String>,
    span_id: String,
    tool_type: Option<String>,
    tool_name: Option<String>,
    started_at: Option<String>,
    input_hash: Option<String>,
    attributes: Value,
}

pub fn inspect_codex_path(path: &Path) -> Result<InspectReport> {
    let files = jsonl_files(path)?;
    let mut report = InspectReport {
        files: files.len(),
        ..InspectReport::default()
    };
    let mut sessions = BTreeSet::new();

    for file in files {
        let lines = read_raw_lines(&file)?;
        for raw in lines {
            if raw.top_type == "session_meta" {
                if let Some(id) = raw.value.pointer("/payload/id").and_then(Value::as_str) {
                    sessions.insert(id.to_string());
                }
            }
            let event_type = normalized_event_type(&raw.top_type, raw.payload_type.as_deref());
            if event_type == "unknown" {
                report.unknown_events += 1;
            }
            report.add_event(event_type, raw.payload_type.as_deref());
        }
    }
    report.sessions = sessions.len();
    Ok(report)
}

pub fn ingest_codex_path(path: &Path, privacy: PrivacyMode) -> Result<ImportBatch> {
    let mut batch = ImportBatch::default();
    for file in jsonl_files(path)? {
        batch.merge(ingest_codex_file(&file, privacy)?);
    }
    Ok(batch)
}

fn ingest_codex_file(path: &Path, privacy: PrivacyMode) -> Result<ImportBatch> {
    let lines = read_raw_lines(path)?;
    if lines.is_empty() {
        return Ok(ImportBatch::default());
    }

    let session_id = session_id_for(path, &lines);
    let trace_id = session_id.clone();
    let first_ts = lines.iter().filter_map(|line| line.timestamp).min();
    let last_ts = lines.iter().filter_map(|line| line.timestamp).max();
    let mut batch = ImportBatch::default();
    let mut turns_by_id: BTreeMap<String, Turn> = BTreeMap::new();
    let mut current_turn_id: Option<String> = None;
    let mut turn_index = 0_i64;
    let mut pending_tools: BTreeMap<String, PendingTool> = BTreeMap::new();
    let mut model: Option<String> = None;
    let mut project_path: Option<String> = None;
    let mut codex_version: Option<String> = None;
    let mut originator: Option<String> = None;
    let mut session_status = "success".to_string();

    for raw in &lines {
        if raw.top_type == "session_meta" {
            let payload = payload(&raw.value);
            project_path = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            codex_version = payload
                .get("cli_version")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            originator = payload
                .get("originator")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        if raw.top_type == "turn_context" {
            let payload = payload(&raw.value);
            model = payload
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or(model);
            project_path = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or(project_path);
        }
    }

    for raw in &lines {
        let payload = payload(&raw.value);
        let timestamp = timestamp_string(raw.timestamp.or(first_ts).unwrap_or_else(Utc::now));
        let mut turn_for_event = current_turn_id.clone();
        let mut span_for_event: Option<String> = None;
        let event_type = normalized_event_type(&raw.top_type, raw.payload_type.as_deref());
        let raw_ref = raw_ref(raw);
        let event_id = stable_id("evt", &[&session_id, &raw_ref, event_type, &timestamp]);
        let mut attributes = base_attributes(raw);

        match (raw.top_type.as_str(), raw.payload_type.as_deref()) {
            ("session_meta", _) => {
                attributes = merge_json(
                    attributes,
                    json!({
                        "source": payload.get("source").and_then(Value::as_str),
                        "cli_version": payload.get("cli_version").and_then(Value::as_str),
                        "originator": payload.get("originator").and_then(Value::as_str),
                        "cwd_hash": payload.get("cwd").and_then(Value::as_str).map(hash_tag),
                        "cwd_display": payload.get("cwd").and_then(Value::as_str).and_then(|p| maybe_path_display(p, privacy)),
                    }),
                );
            }
            ("compacted", _) => {
                attributes = merge_json(attributes, json!({"compacted": true}));
            }
            ("event_msg", Some("task_started")) => {
                let raw_turn = payload
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| stable_id("turn", &[&session_id, &raw_ref]));
                current_turn_id = Some(raw_turn.clone());
                turn_for_event = Some(raw_turn.clone());
                turn_index += 1;
                ensure_turn(
                    &mut turns_by_id,
                    &raw_turn,
                    &session_id,
                    turn_index,
                    Some(timestamp.clone()),
                );
                attributes = merge_json(
                    attributes,
                    json!({
                        "turn_id": raw_turn,
                        "collaboration_mode_kind": payload.get("collaboration_mode_kind").and_then(Value::as_str),
                        "model_context_window": payload.get("model_context_window").and_then(Value::as_i64),
                    }),
                );
            }
            ("event_msg", Some("user_message")) => {
                if current_turn_id.is_none() {
                    turn_index += 1;
                    let inferred = stable_id("turn", &[&session_id, &raw_ref]);
                    current_turn_id = Some(inferred.clone());
                    ensure_turn(
                        &mut turns_by_id,
                        &inferred,
                        &session_id,
                        turn_index,
                        Some(timestamp.clone()),
                    );
                }
                turn_for_event = current_turn_id.clone();
                let message = payload.get("message").and_then(Value::as_str);
                if let Some(turn_id) = &turn_for_event {
                    if let Some(turn) = turns_by_id.get_mut(turn_id) {
                        turn.user_message_hash = message.map(hash_tag);
                    }
                }
                attributes = merge_json(
                    attributes,
                    merge_json(
                        text_attrs("message", message, privacy),
                        json!({
                            "images": payload.get("images").and_then(Value::as_array).map(Vec::len),
                            "local_images": payload.get("local_images").and_then(Value::as_array).map(Vec::len),
                        }),
                    ),
                );
            }
            ("event_msg", Some("agent_message")) => {
                attributes = merge_json(
                    attributes,
                    text_attrs(
                        "message",
                        payload.get("message").and_then(Value::as_str),
                        privacy,
                    ),
                );
            }
            ("event_msg", Some("token_count")) => {
                let usage = usage_from_token_info(payload.get("info"));
                if usage.total_tokens > 0 {
                    let id = stable_id("tok", &[&session_id, &raw_ref]);
                    batch.token_usage.push(TokenUsage {
                        id,
                        session_id: session_id.clone(),
                        turn_id: turn_for_event.clone(),
                        span_id: None,
                        model: model.clone(),
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        cached_tokens: usage.cached_tokens,
                        reasoning_tokens: usage.reasoning_tokens,
                        total_tokens: usage.total_tokens,
                        estimated_cost_usd: None,
                        timestamp: timestamp.clone(),
                    });
                    if let Some(turn_id) = &turn_for_event {
                        if let Some(turn) = turns_by_id.get_mut(turn_id) {
                            turn.input_tokens += usage.input_tokens;
                            turn.output_tokens += usage.output_tokens;
                            turn.cached_tokens += usage.cached_tokens;
                            turn.reasoning_tokens += usage.reasoning_tokens;
                        }
                    }
                }
                attributes = merge_json(attributes, token_attrs(payload.get("info")));
            }
            ("event_msg", Some("task_complete")) => {
                if let Some(raw_turn) = payload.get("turn_id").and_then(Value::as_str) {
                    current_turn_id = Some(raw_turn.to_string());
                    turn_for_event = Some(raw_turn.to_string());
                    ensure_turn(
                        &mut turns_by_id,
                        raw_turn,
                        &session_id,
                        turn_index.max(1),
                        None,
                    );
                    if let Some(turn) = turns_by_id.get_mut(raw_turn) {
                        turn.ended_at = payload
                            .get("completed_at")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                            .or_else(|| Some(timestamp.clone()));
                        turn.duration_ms = payload
                            .get("duration_ms")
                            .and_then(Value::as_i64)
                            .or_else(|| {
                                opt_duration_ms(
                                    turn.started_at.as_deref(),
                                    turn.ended_at.as_deref(),
                                )
                            });
                        turn.status = "success".to_string();
                    }
                }
                attributes = merge_json(
                    attributes,
                    merge_json(
                        text_attrs(
                            "last_agent_message",
                            payload.get("last_agent_message").and_then(Value::as_str),
                            privacy,
                        ),
                        json!({
                            "duration_ms": payload.get("duration_ms").and_then(Value::as_i64),
                            "time_to_first_token_ms": payload.get("time_to_first_token_ms").and_then(Value::as_i64),
                        }),
                    ),
                );
            }
            ("event_msg", Some("turn_aborted")) => {
                session_status = "failed".to_string();
                if let Some(raw_turn) = payload.get("turn_id").and_then(Value::as_str) {
                    current_turn_id = Some(raw_turn.to_string());
                    turn_for_event = Some(raw_turn.to_string());
                    ensure_turn(
                        &mut turns_by_id,
                        raw_turn,
                        &session_id,
                        turn_index.max(1),
                        None,
                    );
                    if let Some(turn) = turns_by_id.get_mut(raw_turn) {
                        turn.ended_at = payload
                            .get("completed_at")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                            .or_else(|| Some(timestamp.clone()));
                        turn.duration_ms = payload
                            .get("duration_ms")
                            .and_then(Value::as_i64)
                            .or_else(|| {
                                opt_duration_ms(
                                    turn.started_at.as_deref(),
                                    turn.ended_at.as_deref(),
                                )
                            });
                        turn.status = "aborted".to_string();
                        turn.error_count += 1;
                    }
                }
                attributes = merge_json(
                    attributes,
                    json!({
                        "reason": payload.get("reason").and_then(Value::as_str),
                        "duration_ms": payload.get("duration_ms").and_then(Value::as_i64),
                    }),
                );
            }
            ("event_msg", Some("error")) => {
                session_status = "failed".to_string();
                if let Some(turn_id) = &turn_for_event {
                    if let Some(turn) = turns_by_id.get_mut(turn_id) {
                        turn.error_count += 1;
                        turn.status = "failed".to_string();
                    }
                }
                attributes = merge_json(
                    attributes,
                    text_attrs(
                        "message",
                        payload.get("message").and_then(Value::as_str),
                        privacy,
                    ),
                );
            }
            ("event_msg" | "response_item", Some("custom_tool_call")) => {
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| stable_id("call", &[&session_id, &raw_ref]));
                let tool_name = payload.get("name").and_then(Value::as_str);
                let input = payload.get("input").and_then(Value::as_str);
                let span_id = stable_id("span", &[&session_id, &call_id]);
                span_for_event = Some(span_id.clone());
                let type_name = tool_type(tool_name, None);
                let attrs = json!({
                    "call_id": call_id,
                    "tool_name": tool_name,
                    "tool_type": type_name,
                    "input_hash": input.map(hash_tag),
                    "status": payload.get("status").and_then(Value::as_str),
                });
                attributes = merge_json(attributes, attrs.clone());
                pending_tools.insert(
                    call_id.clone(),
                    PendingTool {
                        call_id,
                        id: stable_id("tool", &[&session_id, &span_id]),
                        session_id: session_id.clone(),
                        turn_id: turn_for_event.clone(),
                        span_id,
                        tool_type: type_name.map(ToString::to_string),
                        tool_name: tool_name.map(ToString::to_string),
                        started_at: Some(timestamp.clone()),
                        input_hash: input.map(hash_tag),
                        attributes: attrs,
                    },
                );
                if let Some(turn_id) = &turn_for_event {
                    if let Some(turn) = turns_by_id.get_mut(turn_id) {
                        turn.tool_call_count += 1;
                    }
                }
            }
            ("event_msg" | "response_item", Some("custom_tool_call_output")) => {
                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let output_hash = payload.get("output").and_then(Value::as_str).map(hash_tag);
                let span_id = close_tool(
                    &mut batch,
                    &mut pending_tools,
                    &session_id,
                    &trace_id,
                    turn_for_event.clone(),
                    call_id,
                    None,
                    None,
                    &timestamp,
                    "success",
                    None,
                    output_hash.clone(),
                    json!({"call_id": call_id, "output_hash": output_hash}),
                    None,
                );
                span_for_event = Some(span_id);
                attributes = merge_json(
                    attributes,
                    json!({"call_id": call_id, "output_hash": output_hash}),
                );
            }
            ("event_msg", Some("exec_command_end")) => {
                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let exit_code = payload.get("exit_code").and_then(Value::as_i64);
                let status = payload.get("status").and_then(Value::as_str).unwrap_or(
                    if exit_code == Some(0) {
                        "success"
                    } else {
                        "failed"
                    },
                );
                let command = payload.get("command").and_then(Value::as_str);
                let duration_ms = payload.get("duration").and_then(duration_value_ms);
                let stdout_hash = payload.get("stdout").and_then(Value::as_str).map(hash_tag);
                let stderr_hash = payload.get("stderr").and_then(Value::as_str).map(hash_tag);
                let output_hash = payload
                    .get("aggregated_output")
                    .and_then(Value::as_str)
                    .map(hash_tag)
                    .or_else(|| {
                        payload
                            .get("formatted_output")
                            .and_then(Value::as_str)
                            .map(hash_tag)
                    });
                let attrs = merge_json(
                    json!({
                        "call_id": call_id,
                        "tool_name": "exec_command",
                        "tool_type": "shell",
                        "exit_code": exit_code,
                        "cwd_hash": payload.get("cwd").and_then(Value::as_str).map(hash_tag),
                        "cwd_display": payload.get("cwd").and_then(Value::as_str).and_then(|p| maybe_path_display(p, privacy)),
                        "stdout_hash": stdout_hash,
                        "stderr_hash": stderr_hash,
                        "duration_ms": duration_ms,
                    }),
                    command_attrs(command, privacy),
                );
                let span_id = close_tool(
                    &mut batch,
                    &mut pending_tools,
                    &session_id,
                    &trace_id,
                    payload
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| turn_for_event.clone()),
                    call_id,
                    Some("exec_command"),
                    Some("shell"),
                    &timestamp,
                    if status == "completed" {
                        "success"
                    } else {
                        status
                    },
                    exit_code,
                    output_hash,
                    attrs.clone(),
                    duration_ms,
                );
                span_for_event = Some(span_id);
                attributes = merge_json(attributes, attrs);
                if status != "completed" && status != "success" {
                    if let Some(turn_id) = &turn_for_event {
                        if let Some(turn) = turns_by_id.get_mut(turn_id) {
                            turn.error_count += 1;
                        }
                    }
                }
            }
            ("event_msg", Some("patch_apply_end")) => {
                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let success = payload.get("success").and_then(Value::as_bool);
                let status = if success == Some(false) {
                    "failed"
                } else {
                    payload
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("success")
                };
                let stdout_hash = payload.get("stdout").and_then(Value::as_str).map(hash_tag);
                let stderr_hash = payload.get("stderr").and_then(Value::as_str).map(hash_tag);
                let attrs = json!({
                    "call_id": call_id,
                    "tool_name": "apply_patch",
                    "tool_type": "patch",
                    "success": success,
                    "stdout_hash": stdout_hash,
                    "stderr_hash": stderr_hash,
                    "changed_paths": payload.get("changes").and_then(Value::as_array).map(Vec::len),
                });
                let span_id = close_tool(
                    &mut batch,
                    &mut pending_tools,
                    &session_id,
                    &trace_id,
                    payload
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| turn_for_event.clone()),
                    call_id,
                    Some("apply_patch"),
                    Some("patch"),
                    &timestamp,
                    status,
                    None,
                    stdout_hash,
                    attrs.clone(),
                    None,
                );
                span_for_event = Some(span_id);
                attributes = merge_json(attributes, attrs);
            }
            ("event_msg", Some("mcp_tool_call_end")) => {
                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let duration_ms = payload.get("duration").and_then(duration_value_ms);
                let result_hash = payload
                    .get("result")
                    .map(|value| hash_tag(&value.to_string()));
                let invocation_hash = payload
                    .get("invocation")
                    .map(|value| hash_tag(&value.to_string()));
                let attrs = json!({
                    "call_id": call_id,
                    "tool_type": "mcp",
                    "tool_name": "mcp_tool",
                    "duration_ms": duration_ms,
                    "result_hash": result_hash,
                    "invocation_hash": invocation_hash,
                });
                let span_id = close_tool(
                    &mut batch,
                    &mut pending_tools,
                    &session_id,
                    &trace_id,
                    turn_for_event.clone(),
                    call_id,
                    Some("mcp_tool"),
                    Some("mcp"),
                    &timestamp,
                    "success",
                    None,
                    result_hash,
                    attrs.clone(),
                    duration_ms,
                );
                span_for_event = Some(span_id);
                attributes = merge_json(attributes, attrs);
            }
            ("event_msg" | "response_item", Some("web_search_call")) => {
                let call_id = stable_id("call", &[&session_id, &raw_ref]);
                let span_id = stable_id("span", &[&session_id, &call_id]);
                span_for_event = Some(span_id.clone());
                let attrs = json!({
                    "call_id": call_id,
                    "tool_name": "web_search",
                    "tool_type": "web",
                    "action": payload.get("action").and_then(Value::as_str),
                    "status": payload.get("status").and_then(Value::as_str),
                });
                pending_tools.insert(
                    call_id.clone(),
                    PendingTool {
                        call_id,
                        id: stable_id("tool", &[&session_id, &span_id]),
                        session_id: session_id.clone(),
                        turn_id: turn_for_event.clone(),
                        span_id,
                        tool_type: Some("web".to_string()),
                        tool_name: Some("web_search".to_string()),
                        started_at: Some(timestamp.clone()),
                        input_hash: None,
                        attributes: attrs.clone(),
                    },
                );
                attributes = merge_json(attributes, attrs);
                if let Some(turn_id) = &turn_for_event {
                    if let Some(turn) = turns_by_id.get_mut(turn_id) {
                        turn.tool_call_count += 1;
                    }
                }
            }
            ("event_msg", Some("web_search_end")) => {
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("web_search");
                let query_hash = payload.get("query").and_then(Value::as_str).map(hash_tag);
                let attrs = json!({
                    "call_id": call_id,
                    "tool_name": "web_search",
                    "tool_type": "web",
                    "action": payload.get("action").and_then(Value::as_str),
                    "query_hash": query_hash,
                });
                let span_id = close_tool(
                    &mut batch,
                    &mut pending_tools,
                    &session_id,
                    &trace_id,
                    turn_for_event.clone(),
                    call_id,
                    Some("web_search"),
                    Some("web"),
                    &timestamp,
                    "success",
                    None,
                    query_hash,
                    attrs.clone(),
                    None,
                );
                span_for_event = Some(span_id);
                attributes = merge_json(attributes, attrs);
            }
            ("event_msg", Some("context_compacted")) => {
                attributes = merge_json(attributes, json!({"compacted": true}));
            }
            ("event_msg", Some("thread_name_updated")) => {
                attributes = merge_json(
                    attributes,
                    json!({
                        "thread_id_hash": payload.get("thread_id").and_then(Value::as_str).map(hash_tag),
                        "thread_name_hash": payload.get("thread_name").and_then(Value::as_str).map(hash_tag),
                    }),
                );
            }
            ("turn_context", _) => {
                let raw_turn = payload.get("turn_id").and_then(Value::as_str);
                if let Some(raw_turn) = raw_turn {
                    if current_turn_id.as_deref() != Some(raw_turn) {
                        turn_index += 1;
                    }
                    current_turn_id = Some(raw_turn.to_string());
                    turn_for_event = Some(raw_turn.to_string());
                    ensure_turn(
                        &mut turns_by_id,
                        raw_turn,
                        &session_id,
                        turn_index.max(1),
                        Some(timestamp.clone()),
                    );
                }
                model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or(model.clone());
                attributes = merge_json(
                    attributes,
                    json!({
                        "model": payload.get("model").and_then(Value::as_str),
                        "cwd_hash": payload.get("cwd").and_then(Value::as_str).map(hash_tag),
                        "cwd_display": payload.get("cwd").and_then(Value::as_str).and_then(|p| maybe_path_display(p, privacy)),
                    }),
                );
            }
            ("response_item", Some("function_call")) => {
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| stable_id("call", &[&session_id, &raw_ref]));
                let tool_name = payload.get("name").and_then(Value::as_str);
                let span_id = stable_id("span", &[&session_id, &call_id]);
                span_for_event = Some(span_id.clone());
                let args_text = payload.get("arguments").and_then(Value::as_str);
                let args_json = args_text
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .unwrap_or(Value::Null);
                let command = args_json.get("cmd").and_then(Value::as_str);
                let tool_type = tool_type(tool_name, command);
                let attrs = merge_json(
                    json!({
                        "call_id": call_id,
                        "tool_name": tool_name,
                        "tool_type": tool_type,
                        "arguments_hash": args_text.map(hash_tag),
                        "workdir_hash": args_json.get("workdir").and_then(Value::as_str).map(hash_tag),
                        "workdir_display": args_json.get("workdir").and_then(Value::as_str).and_then(|p| maybe_path_display(p, privacy)),
                    }),
                    command_attrs(command, privacy),
                );
                attributes = merge_json(attributes, attrs.clone());
                pending_tools.insert(
                    call_id.clone(),
                    PendingTool {
                        call_id,
                        id: stable_id("tool", &[&session_id, &span_id]),
                        session_id: session_id.clone(),
                        turn_id: turn_for_event.clone(),
                        span_id,
                        tool_type: tool_type.map(ToString::to_string),
                        tool_name: tool_name.map(ToString::to_string),
                        started_at: Some(timestamp.clone()),
                        input_hash: args_text.map(hash_tag),
                        attributes: attrs,
                    },
                );
                if let Some(turn_id) = &turn_for_event {
                    if let Some(turn) = turns_by_id.get_mut(turn_id) {
                        turn.tool_call_count += 1;
                    }
                }
            }
            ("response_item", Some("function_call_output")) => {
                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let output = payload.get("output").and_then(Value::as_str);
                let exit_code = output.and_then(extract_exit_code);
                let status = match exit_code {
                    Some(0) | None => "success",
                    Some(_) => "failed",
                };
                let output_hash = output.map(hash_tag);
                attributes = merge_json(
                    attributes,
                    json!({
                        "call_id": call_id,
                        "exit_code": exit_code,
                        "output_hash": output_hash,
                        "output_hidden": privacy != PrivacyMode::None,
                    }),
                );
                if let Some(mut pending) = pending_tools.remove(call_id) {
                    span_for_event = Some(pending.span_id.clone());
                    let duration_ms =
                        opt_duration_ms(pending.started_at.as_deref(), Some(&timestamp));
                    let attrs = merge_json(
                        pending.attributes.clone(),
                        json!({
                            "exit_code": exit_code,
                            "output_hash": output_hash,
                            "call_id": pending.call_id,
                        }),
                    );
                    batch.tool_calls.insert(
                        pending.id.clone(),
                        ToolCall {
                            id: pending.id,
                            session_id: pending.session_id,
                            turn_id: pending.turn_id.clone(),
                            span_id: pending.span_id.clone(),
                            tool_type: pending.tool_type.take(),
                            tool_name: pending.tool_name.take(),
                            started_at: pending.started_at.clone(),
                            ended_at: Some(timestamp.clone()),
                            duration_ms,
                            status: status.to_string(),
                            exit_code,
                            input_hash: pending.input_hash.take(),
                            output_hash,
                            error_type: (status == "failed")
                                .then_some("shell_exit_nonzero".to_string()),
                            attributes: attrs.clone(),
                        },
                    );
                    batch.spans.insert(
                        pending.span_id.clone(),
                        Span {
                            id: pending.span_id,
                            trace_id: trace_id.clone(),
                            parent_span_id: pending
                                .turn_id
                                .as_ref()
                                .map(|id| turn_span_id(&session_id, id)),
                            session_id: session_id.clone(),
                            turn_id: pending.turn_id,
                            span_type: "tool_call".to_string(),
                            name: attrs
                                .get("tool_name")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                            started_at: pending.started_at,
                            ended_at: Some(timestamp.clone()),
                            duration_ms,
                            status: status.to_string(),
                            attributes: attrs,
                        },
                    );
                }
                if status == "failed" {
                    if let Some(turn_id) = &turn_for_event {
                        if let Some(turn) = turns_by_id.get_mut(turn_id) {
                            turn.error_count += 1;
                        }
                    }
                }
            }
            ("response_item", Some("message")) => {
                attributes = merge_json(
                    attributes,
                    json!({
                        "role": payload.get("role").and_then(Value::as_str),
                        "content_items": payload.get("content").and_then(Value::as_array).map(Vec::len),
                        "phase": payload.get("phase").and_then(Value::as_str),
                    }),
                );
            }
            ("response_item", Some("reasoning")) => {
                attributes = merge_json(
                    attributes,
                    json!({
                        "summary_items": payload.get("summary").and_then(Value::as_array).map(Vec::len),
                        "content_items": payload.get("content").and_then(Value::as_array).map(Vec::len),
                        "encrypted_content_present": payload.get("encrypted_content").is_some(),
                    }),
                );
            }
            _ => {
                batch.unknown_events.push(UnknownEvent {
                    id: stable_id("unk", &[&session_id, &raw_ref]),
                    raw_event: stored_raw_event(raw, privacy),
                    reason: format!(
                        "unmapped codex event: {} / {:?}",
                        raw.top_type, raw.payload_type
                    ),
                });
            }
        }

        batch.events.push(Event {
            id: event_id,
            session_id: session_id.clone(),
            turn_id: turn_for_event,
            span_id: span_for_event,
            event_type: event_type.to_string(),
            timestamp,
            attributes,
            raw_event: stored_raw_event(raw, privacy),
            raw_source: SOURCE_CODEX_JSONL.to_string(),
            raw_ref,
        });
    }

    for pending in pending_tools.into_values() {
        let ended_at = last_ts.map(timestamp_string);
        let duration_ms = opt_duration_ms(pending.started_at.as_deref(), ended_at.as_deref());
        batch.tool_calls.insert(
            pending.id.clone(),
            ToolCall {
                id: pending.id,
                session_id: pending.session_id.clone(),
                turn_id: pending.turn_id.clone(),
                span_id: pending.span_id.clone(),
                tool_type: pending.tool_type.clone(),
                tool_name: pending.tool_name.clone(),
                started_at: pending.started_at.clone(),
                ended_at: ended_at.clone(),
                duration_ms,
                status: "unknown".to_string(),
                exit_code: None,
                input_hash: pending.input_hash,
                output_hash: None,
                error_type: None,
                attributes: pending.attributes.clone(),
            },
        );
        batch.spans.insert(
            pending.span_id.clone(),
            Span {
                id: pending.span_id,
                trace_id: trace_id.clone(),
                parent_span_id: pending
                    .turn_id
                    .as_ref()
                    .map(|id| turn_span_id(&session_id, id)),
                session_id: pending.session_id,
                turn_id: pending.turn_id,
                span_type: "tool_call".to_string(),
                name: pending.tool_name,
                started_at: pending.started_at,
                ended_at,
                duration_ms,
                status: "unknown".to_string(),
                attributes: pending.attributes,
            },
        );
    }

    let session_started = first_ts.map(timestamp_string);
    let session_ended = last_ts.map(timestamp_string);
    for turn in turns_by_id.values_mut() {
        if turn.ended_at.is_none() {
            turn.ended_at = session_ended.clone();
        }
        turn.duration_ms = opt_duration_ms(turn.started_at.as_deref(), turn.ended_at.as_deref());
        batch.spans.insert(
            turn_span_id(&session_id, &turn.id),
            Span {
                id: turn_span_id(&session_id, &turn.id),
                trace_id: trace_id.clone(),
                parent_span_id: Some(session_span_id(&session_id)),
                session_id: session_id.clone(),
                turn_id: Some(turn.id.clone()),
                span_type: "turn".to_string(),
                name: Some(format!("turn {}", turn.turn_index)),
                started_at: turn.started_at.clone(),
                ended_at: turn.ended_at.clone(),
                duration_ms: turn.duration_ms,
                status: turn.status.clone(),
                attributes: json!({"turn_index": turn.turn_index}),
            },
        );
    }

    let (project_id, project_path_hash) = project_path
        .as_deref()
        .map(|path| (Some(short_project_id(path)), Some(hash_tag(path))))
        .unwrap_or((None, None));
    batch.sessions.insert(
        session_id.clone(),
        Session {
            id: session_id.clone(),
            source: SOURCE_CODEX_JSONL.to_string(),
            project_id,
            project_path_hash,
            started_at: session_started.clone(),
            ended_at: session_ended.clone(),
            duration_ms: opt_duration_ms(session_started.as_deref(), session_ended.as_deref()),
            status: session_status.clone(),
            model,
            codex_version,
            auth_mode: None,
            originator,
            raw_file_path: Some(path.to_string_lossy().to_string()),
        },
    );
    batch.spans.insert(
        session_span_id(&session_id),
        Span {
            id: session_span_id(&session_id),
            trace_id,
            parent_span_id: None,
            session_id,
            turn_id: None,
            span_type: "session".to_string(),
            name: Some("session".to_string()),
            started_at: session_started,
            ended_at: session_ended,
            duration_ms: opt_duration_ms(
                first_ts.map(timestamp_string).as_deref(),
                last_ts.map(timestamp_string).as_deref(),
            ),
            status: session_status,
            attributes: json!({}),
        },
    );
    batch.turns = turns_by_id;
    Ok(batch)
}

fn read_raw_lines(path: &Path) -> Result<Vec<RawLine>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read {}:{}", path.display(), idx + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)
            .with_context(|| format!("parse json {}:{}", path.display(), idx + 1))?;
        let top_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let payload_type = value
            .pointer("/payload/type")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp);
        lines.push(RawLine {
            path: path.to_path_buf(),
            line_no: idx + 1,
            value,
            top_type,
            payload_type,
            timestamp,
        });
    }
    Ok(lines)
}

fn jsonl_files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path.to_path_buf());
        }
        return Ok(files);
    }
    visit_jsonl(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_jsonl(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("read dir {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_jsonl(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn payload(value: &Value) -> &Map<String, Value> {
    if let Some(payload) = value.get("payload").and_then(Value::as_object) {
        payload
    } else {
        empty_object()
    }
}

fn empty_object() -> &'static Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Map::new)
}

fn session_id_for(path: &Path, lines: &[RawLine]) -> String {
    lines
        .iter()
        .find_map(|line| {
            (line.top_type == "session_meta")
                .then(|| line.value.pointer("/payload/id").and_then(Value::as_str))
                .flatten()
        })
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session");
            stable_id("ses", &[stem, &path.to_string_lossy()])
        })
}

fn ensure_turn(
    turns: &mut BTreeMap<String, Turn>,
    id: &str,
    session_id: &str,
    turn_index: i64,
    started_at: Option<String>,
) {
    turns.entry(id.to_string()).or_insert_with(|| Turn {
        id: id.to_string(),
        session_id: session_id.to_string(),
        turn_index,
        started_at,
        ended_at: None,
        duration_ms: None,
        status: "success".to_string(),
        user_message_hash: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        reasoning_tokens: 0,
        tool_call_count: 0,
        error_count: 0,
    });
}

fn normalized_event_type(top_type: &str, payload_type: Option<&str>) -> &'static str {
    match (top_type, payload_type) {
        ("session_meta", _) => "session.started",
        ("compacted", _) => "context.compacted",
        ("turn_context", _) => "turn.context",
        ("event_msg", Some("task_started")) => "turn.started",
        ("event_msg", Some("user_message")) => "user.message",
        ("event_msg", Some("agent_message")) => "agent.message",
        ("event_msg", Some("token_count")) => "token.usage",
        ("event_msg", Some("task_complete")) => "turn.ended",
        ("event_msg", Some("turn_aborted")) => "turn.ended",
        ("event_msg", Some("error")) => "error",
        ("event_msg" | "response_item", Some("custom_tool_call")) => "tool.call.started",
        ("event_msg" | "response_item", Some("custom_tool_call_output")) => "tool.call.ended",
        ("event_msg", Some("exec_command_end")) => "shell.command.ended",
        ("event_msg", Some("patch_apply_end")) => "patch.apply.ended",
        ("event_msg", Some("mcp_tool_call_end")) => "mcp.call.ended",
        ("event_msg" | "response_item", Some("web_search_call")) => "tool.call.started",
        ("event_msg", Some("web_search_end")) => "tool.call.ended",
        ("event_msg", Some("context_compacted")) => "context.compacted",
        ("event_msg", Some("thread_name_updated")) => "session.metadata_updated",
        ("response_item", Some("function_call")) => "tool.call.started",
        ("response_item", Some("function_call_output")) => "tool.call.ended",
        ("response_item", Some("message")) => "model.message",
        ("response_item", Some("reasoning")) => "model.reasoning",
        _ => "unknown",
    }
}

fn base_attributes(raw: &RawLine) -> Value {
    json!({
        "codex_top_type": raw.top_type,
        "codex_payload_type": raw.payload_type,
        "line_no": raw.line_no,
    })
}

fn stored_raw_event(raw: &RawLine, privacy: PrivacyMode) -> Value {
    if privacy == PrivacyMode::None {
        return raw.value.clone();
    }
    json!({
        "timestamp": raw.value.get("timestamp").and_then(Value::as_str),
        "type": raw.top_type,
        "payload_type": raw.payload_type,
        "line_no": raw.line_no,
        "redacted": true,
    })
}

fn token_attrs(info: Option<&Value>) -> Value {
    let total = info.and_then(|info| info.get("total_token_usage"));
    json!({
        "input_tokens_total": token_field(total, "input_tokens"),
        "output_tokens_total": token_field(total, "output_tokens"),
        "cached_tokens_total": token_field(total, "cached_input_tokens"),
        "reasoning_tokens_total": token_field(total, "reasoning_output_tokens"),
        "total_tokens_so_far": token_field(total, "total_tokens"),
        "model_context_window": info.and_then(|info| info.get("model_context_window")).and_then(Value::as_i64),
    })
}

fn usage_from_token_info(info: Option<&Value>) -> TokenFields {
    let last = info.and_then(|info| info.get("last_token_usage"));
    TokenFields {
        input_tokens: token_field(last, "input_tokens"),
        output_tokens: token_field(last, "output_tokens"),
        cached_tokens: token_field(last, "cached_input_tokens"),
        reasoning_tokens: token_field(last, "reasoning_output_tokens"),
        total_tokens: token_field(last, "total_tokens"),
    }
}

#[derive(Debug, Default)]
struct TokenFields {
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
}

fn token_field(value: Option<&Value>, field: &str) -> i64 {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn tool_type(tool_name: Option<&str>, command: Option<&str>) -> Option<&'static str> {
    match (tool_name, command) {
        (Some("exec_command"), _) => Some("shell"),
        (Some("apply_patch"), _) => Some("patch"),
        (Some("view_image"), _) => Some("file"),
        (Some(_), Some(_)) => Some("shell"),
        (Some(_), None) => Some("tool"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn close_tool(
    batch: &mut ImportBatch,
    pending_tools: &mut BTreeMap<String, PendingTool>,
    session_id: &str,
    trace_id: &str,
    fallback_turn_id: Option<String>,
    call_id: &str,
    fallback_tool_name: Option<&str>,
    fallback_tool_type: Option<&str>,
    ended_at: &str,
    status: &str,
    exit_code: Option<i64>,
    output_hash: Option<String>,
    extra_attrs: Value,
    duration_override_ms: Option<i64>,
) -> String {
    let pending = pending_tools.remove(call_id);
    let span_id = pending
        .as_ref()
        .map(|pending| pending.span_id.clone())
        .unwrap_or_else(|| stable_id("span", &[session_id, call_id]));
    let id = pending
        .as_ref()
        .map(|pending| pending.id.clone())
        .unwrap_or_else(|| stable_id("tool", &[session_id, &span_id]));
    let turn_id = pending
        .as_ref()
        .and_then(|pending| pending.turn_id.clone())
        .or(fallback_turn_id);
    let started_at = pending
        .as_ref()
        .and_then(|pending| pending.started_at.clone());
    let duration_ms =
        duration_override_ms.or_else(|| opt_duration_ms(started_at.as_deref(), Some(ended_at)));
    let tool_name = pending
        .as_ref()
        .and_then(|pending| pending.tool_name.clone())
        .or_else(|| fallback_tool_name.map(ToString::to_string));
    let tool_type = pending
        .as_ref()
        .and_then(|pending| pending.tool_type.clone())
        .or_else(|| fallback_tool_type.map(ToString::to_string));
    let input_hash = pending
        .as_ref()
        .and_then(|pending| pending.input_hash.clone());
    let base_attrs = pending
        .as_ref()
        .map(|pending| pending.attributes.clone())
        .unwrap_or_else(|| {
            json!({
                "call_id": call_id,
                "tool_name": tool_name,
                "tool_type": tool_type,
            })
        });
    let attributes = merge_json(base_attrs, extra_attrs);

    batch.tool_calls.insert(
        id.clone(),
        ToolCall {
            id,
            session_id: session_id.to_string(),
            turn_id: turn_id.clone(),
            span_id: span_id.clone(),
            tool_type,
            tool_name: tool_name.clone(),
            started_at: started_at.clone(),
            ended_at: Some(ended_at.to_string()),
            duration_ms,
            status: status.to_string(),
            exit_code,
            input_hash,
            output_hash,
            error_type: (status == "failed").then_some("tool_error".to_string()),
            attributes: attributes.clone(),
        },
    );
    batch.spans.insert(
        span_id.clone(),
        Span {
            id: span_id.clone(),
            trace_id: trace_id.to_string(),
            parent_span_id: turn_id.as_ref().map(|id| turn_span_id(session_id, id)),
            session_id: session_id.to_string(),
            turn_id,
            span_type: "tool_call".to_string(),
            name: tool_name,
            started_at,
            ended_at: Some(ended_at.to_string()),
            duration_ms,
            status: status.to_string(),
            attributes,
        },
    );
    span_id
}

fn duration_value_ms(value: &Value) -> Option<i64> {
    let secs = value.get("secs").and_then(Value::as_i64).unwrap_or(0);
    let nanos = value.get("nanos").and_then(Value::as_i64).unwrap_or(0);
    Some(secs.saturating_mul(1000) + nanos / 1_000_000)
}

fn extract_exit_code(output: &str) -> Option<i64> {
    let marker = "Process exited with code ";
    let start = output.find(marker)? + marker.len();
    let code = output[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .collect::<String>();
    code.parse().ok()
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

fn timestamp_string(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn opt_duration_ms(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let start = parse_timestamp(start?)?;
    let end = parse_timestamp(end?)?;
    Some(end.signed_duration_since(start).num_milliseconds().max(0))
}

fn raw_ref(raw: &RawLine) -> String {
    format!("{}:{}", raw.path.display(), raw.line_no)
}

fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let joined = parts.join("\x1f");
    format!(
        "{prefix}_{}",
        sha256_hex(&joined).chars().take(24).collect::<String>()
    )
}

fn session_span_id(session_id: &str) -> String {
    stable_id("span", &[session_id, "session"])
}

fn turn_span_id(session_id: &str, turn_id: &str) -> String {
    stable_id("span", &[session_id, turn_id])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_exit_code() {
        assert_eq!(
            extract_exit_code("Process exited with code 0\nOutput:"),
            Some(0)
        );
        assert_eq!(extract_exit_code("Process exited with code 101"), Some(101));
        assert_eq!(extract_exit_code("no code"), None);
    }
}
