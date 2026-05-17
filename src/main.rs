use anyhow::{Context, Result, bail};
use atrail::api;
use atrail::{PrivacyMode, Store, ingest_codex_path, inspect_codex_path};
use serde::Serialize;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };

    match command {
        "inspect" => {
            let opts = Args::new(&args[1..]);
            let source = opts
                .positionals
                .first()
                .map(expand_tilde)
                .unwrap_or_else(default_codex_sessions);
            let report = inspect_codex_path(&source)?;
            print_json(&report)?;
        }
        "ingest" => {
            let opts = Args::new(&args[1..]);
            let source = opts
                .positionals
                .first()
                .map(expand_tilde)
                .unwrap_or_else(default_codex_sessions);
            let db = opts
                .value("--db")
                .map(expand_tilde)
                .unwrap_or_else(default_db_path);
            let privacy = opts
                .value("--privacy")
                .and_then(PrivacyMode::parse)
                .unwrap_or(PrivacyMode::Safe);
            let batch = ingest_codex_path(&source, privacy)?;
            let mut store = Store::open(&db)?;
            store.insert_batch(&batch)?;
            print_json(&serde_json::json!({
                "db": db,
                "sessions": batch.sessions.len(),
                "turns": batch.turns.len(),
                "events": batch.events.len(),
                "spans": batch.spans.len(),
                "token_usage": batch.token_usage.len(),
                "tool_calls": batch.tool_calls.len(),
                "unknown_events": batch.unknown_events.len()
            }))?;
        }
        "sessions" => {
            let opts = Args::new(&args[1..]);
            let db = opts
                .value("--db")
                .map(expand_tilde)
                .unwrap_or_else(default_db_path);
            let limit = opts.value("--limit").unwrap_or("50").parse().unwrap_or(50);
            let store = Store::open(&db)?;
            print_json(&store.sessions(limit)?)?;
        }
        "timeline" => {
            let opts = Args::new(&args[1..]);
            let Some(session_id) = opts.positionals.first() else {
                bail!("timeline requires a session id");
            };
            let db = opts
                .value("--db")
                .map(expand_tilde)
                .unwrap_or_else(default_db_path);
            let store = Store::open(&db)?;
            print_json(&store.timeline(session_id)?)?;
        }
        "summary" => {
            let opts = Args::new(&args[1..]);
            let db = opts
                .value("--db")
                .map(expand_tilde)
                .unwrap_or_else(default_db_path);
            let store = Store::open(&db)?;
            print_json(&store.metrics_summary()?)?;
        }
        "serve" => {
            let opts = Args::new(&args[1..]);
            let db = opts
                .value("--db")
                .map(expand_tilde)
                .unwrap_or_else(default_db_path);
            let host = opts.value("--host").unwrap_or("127.0.0.1");
            let port = opts
                .value("--port")
                .unwrap_or("4319")
                .parse()
                .unwrap_or(4319);
            api::serve(db, host, port)?;
        }
        "doctor" => {
            print_json(&serde_json::json!({
                "default_source": default_codex_sessions(),
                "default_db": default_db_path(),
                "privacy_default": "safe"
            }))?;
        }
        "-h" | "--help" | "help" => print_help(),
        other => bail!("unknown command: {other}"),
    }
    Ok(())
}

struct Args {
    positionals: Vec<String>,
    pairs: Vec<(String, String)>,
}

impl Args {
    fn new(args: &[String]) -> Self {
        let mut positionals = Vec::new();
        let mut pairs = Vec::new();
        let mut idx = 0;
        while idx < args.len() {
            let arg = &args[idx];
            if arg.starts_with("--") {
                let value = args
                    .get(idx + 1)
                    .cloned()
                    .unwrap_or_else(|| "true".to_string());
                pairs.push((arg.clone(), value));
                idx += 2;
            } else {
                positionals.push(arg.clone());
                idx += 1;
            }
        }
        Self { positionals, pairs }
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_help() {
    println!(
        r#"atrail - Agent Session Trail Observatory

USAGE:
  atrail inspect [SOURCE]
  atrail ingest [SOURCE] [--db PATH] [--privacy safe|strict|none]
  atrail sessions [--db PATH] [--limit N]
  atrail timeline SESSION_ID [--db PATH]
  atrail summary [--db PATH]
  atrail serve [--db PATH] [--host 127.0.0.1] [--port 4319]
  atrail doctor

Defaults:
  source: ~/.codex/sessions
  db:     ~/.atrail/atrail.db

Web UI:
  atrail serve opens the dashboard at http://HOST:PORT/
  JSON API remains available under /api
"#
    );
}

fn default_codex_sessions() -> PathBuf {
    expand_tilde("~/.codex/sessions")
}

fn default_db_path() -> PathBuf {
    expand_tilde("~/.atrail/atrail.db")
}

fn expand_tilde(value: impl AsRef<str>) -> PathBuf {
    let value = value.as_ref();
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    Path::new(value).to_path_buf()
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
        .unwrap_or_else(|_| PathBuf::from("."))
}
