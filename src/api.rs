use crate::pricing::{estimate_cost, openai_pricing_catalog};
use crate::storage::Store;
use crate::ui;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

pub fn serve(db_path: PathBuf, host: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind((host, port))
        .with_context(|| format!("bind api server on {host}:{port}"))?;
    eprintln!("atrail api listening on http://{host}:{port}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, db_path.clone()) {
                    eprintln!("request failed: {err:#}");
                }
            }
            Err(err) => eprintln!("accept failed: {err}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, db_path: PathBuf) -> Result<()> {
    let mut buffer = [0; 4096];
    let size = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or("/");

    if method != "GET" {
        return respond_json(stream, 405, &json!({"error": "method not allowed"}));
    }

    let path = path.split('?').next().unwrap_or(path);
    if let Some(asset) = ui::asset(path) {
        return respond_text(stream, 200, asset.content_type, asset.body);
    }

    let store = Store::open(&db_path)?;
    match route(path, &store) {
        Ok(body) => respond_json(stream, 200, &body),
        Err(ApiError::NotFound) => respond_json(stream, 404, &json!({"error": "not found"})),
        Err(ApiError::Anyhow(err)) => respond_json(stream, 500, &json!({"error": err.to_string()})),
    }
}

fn route(path: &str, store: &Store) -> Result<serde_json::Value, ApiError> {
    if path == "/api" {
        return Ok(json!({
            "service": "atrail",
            "endpoints": [
                "/api/sessions",
                "/api/sessions/{id}/timeline",
                "/api/metrics/summary",
                "/api/pricing/openai",
                "/api/admin/status"
            ]
        }));
    }
    if path == "/api/sessions" {
        return priced_sessions(store);
    }
    if path == "/api/metrics/summary" {
        return priced_metrics_summary(store);
    }
    if path == "/api/pricing/openai" {
        return to_value(openai_pricing_catalog());
    }
    if path == "/api/admin/status" {
        return Ok(json!({"status": "ok"}));
    }
    if let Some(rest) = path.strip_prefix("/api/sessions/") {
        if let Some(id) = rest.strip_suffix("/timeline") {
            return to_value(store.timeline(id)?);
        }
        return priced_session(store, rest);
    }
    Err(ApiError::NotFound)
}

fn priced_sessions(store: &Store) -> Result<Value, ApiError> {
    let catalog = openai_pricing_catalog();
    let mut sessions = store.sessions(100)?;
    for session in &mut sessions {
        if let Some(estimate) = estimate_cost(
            session.model.as_deref(),
            session.input_tokens,
            session.cached_tokens,
            session.output_tokens,
            session.reasoning_tokens,
            &catalog,
        ) {
            session.estimated_cost_usd = Some(estimate.total_usd);
            session.input_cost_usd = Some(estimate.input_usd);
            session.cached_input_cost_usd = Some(estimate.cached_input_usd);
            session.output_cost_usd = Some(estimate.output_usd);
            session.cost_model = Some(estimate.model);
            session.cost_source = Some(estimate.price_source);
        }
    }
    to_value(sessions)
}

fn priced_session(store: &Store, id: &str) -> Result<Value, ApiError> {
    let catalog = openai_pricing_catalog();
    let Some(mut session) = store.session(id)? else {
        return Err(ApiError::NotFound);
    };
    if let Some(estimate) = estimate_cost(
        session.model.as_deref(),
        session.input_tokens,
        session.cached_tokens,
        session.output_tokens,
        session.reasoning_tokens,
        &catalog,
    ) {
        session.estimated_cost_usd = Some(estimate.total_usd);
        session.input_cost_usd = Some(estimate.input_usd);
        session.cached_input_cost_usd = Some(estimate.cached_input_usd);
        session.output_cost_usd = Some(estimate.output_usd);
        session.cost_model = Some(estimate.model);
        session.cost_source = Some(estimate.price_source);
    }
    to_value(session)
}

fn priced_metrics_summary(store: &Store) -> Result<Value, ApiError> {
    let catalog = openai_pricing_catalog();
    let usage = store.token_usage_by_model()?;
    let mut estimated_cost_usd = 0.0;
    let mut priced_tokens = 0_i64;
    let mut unpriced_tokens = 0_i64;

    for row in usage {
        if let Some(estimate) = estimate_cost(
            row.model.as_deref(),
            row.input_tokens,
            row.cached_tokens,
            row.output_tokens,
            row.reasoning_tokens,
            &catalog,
        ) {
            estimated_cost_usd += estimate.total_usd;
            priced_tokens += estimate.priced_tokens;
        } else {
            unpriced_tokens += row.input_tokens + row.output_tokens;
        }
    }

    let mut value = to_value(store.metrics_summary()?)?;
    if let Value::Object(ref mut object) = value {
        object.insert(
            "estimated_cost_usd".to_string(),
            json!(round_cost(estimated_cost_usd)),
        );
        object.insert("priced_tokens".to_string(), json!(priced_tokens));
        object.insert("unpriced_tokens".to_string(), json!(unpriced_tokens));
        object.insert(
            "pricing".to_string(),
            Value::Object(pricing_summary(&catalog)),
        );
    }
    Ok(value)
}

fn pricing_summary(catalog: &crate::pricing::PricingCatalog) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("provider".to_string(), json!(&catalog.provider));
    object.insert("source_url".to_string(), json!(&catalog.source_url));
    object.insert("fetched_at".to_string(), json!(&catalog.fetched_at));
    object.insert("status".to_string(), json!(&catalog.status));
    object.insert("tier".to_string(), json!(&catalog.tier));
    object.insert("context".to_string(), json!(&catalog.context));
    object.insert("currency".to_string(), json!(&catalog.currency));
    object.insert("unit".to_string(), json!(&catalog.unit));
    object.insert("error".to_string(), json!(&catalog.error));
    object
}

fn round_cost(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn to_value<T: Serialize>(value: T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(|err| ApiError::Anyhow(err.into()))
}

fn respond_json<T: Serialize>(mut stream: TcpStream, status: u16, body: &T) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let body = serde_json::to_string_pretty(body)?;
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    Ok(())
}

fn respond_text(mut stream: TcpStream, status: u16, content_type: &str, body: &str) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    Ok(())
}

enum ApiError {
    NotFound,
    Anyhow(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self::Anyhow(value)
    }
}
