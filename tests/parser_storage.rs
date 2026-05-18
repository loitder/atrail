use atrail::{PrivacyMode, Store, ingest_codex_path};
use std::fs;

#[test]
fn ingests_fixture_into_sqlite() {
    let source = fixture_dir();
    let batch = ingest_codex_path(&source, PrivacyMode::Safe).unwrap();
    assert_eq!(batch.sessions.len(), 1);
    assert_eq!(batch.turns.len(), 1);
    assert_eq!(batch.tool_calls.len(), 1);
    assert_eq!(batch.token_usage.len(), 1);

    let db = std::env::temp_dir().join(format!(
        "atrail-test-{}-{}.db",
        std::process::id(),
        "ingests_fixture"
    ));
    let _ = fs::remove_file(&db);
    let mut store = Store::open(&db).unwrap();
    store.insert_batch(&batch).unwrap();
    store.insert_batch(&batch).unwrap();

    let sessions = store.sessions(10).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].total_tokens, 14);
    assert_eq!(sessions[0].tool_call_count, 1);

    let timeline = store.timeline("ses_fixture").unwrap();
    assert!(
        timeline
            .iter()
            .any(|item| item.event_type == "tool.call.started")
    );
    assert!(timeline.iter().any(|item| item.event_type == "token.usage"));
    let _ = fs::remove_file(&db);
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}
