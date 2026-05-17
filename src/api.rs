use crate::storage::Store;
use crate::ui;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;
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
                "/api/admin/status"
            ]
        }));
    }
    if path == "/api/sessions" {
        return to_value(store.sessions(100)?);
    }
    if path == "/api/metrics/summary" {
        return to_value(store.metrics_summary()?);
    }
    if path == "/api/admin/status" {
        return Ok(json!({"status": "ok"}));
    }
    if let Some(rest) = path.strip_prefix("/api/sessions/") {
        if let Some(id) = rest.strip_suffix("/timeline") {
            return to_value(store.timeline(id)?);
        }
        return store
            .session(rest)?
            .map_or(Err(ApiError::NotFound), to_value);
    }
    Err(ApiError::NotFound)
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
