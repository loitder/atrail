use crate::model::PrivacyMode;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn hash_tag(value: &str) -> String {
    format!("sha256:{}", sha256_hex(value))
}

pub fn short_project_id(path: &str) -> String {
    sha256_hex(path).chars().take(12).collect()
}

pub fn maybe_path_display(path: &str, mode: PrivacyMode) -> Option<String> {
    match mode {
        PrivacyMode::None => Some(path.to_string()),
        PrivacyMode::Safe => Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string()),
        PrivacyMode::Strict => None,
    }
}

pub fn text_attrs(field: &str, text: Option<&str>, mode: PrivacyMode) -> Value {
    let Some(text) = text else {
        return json!({});
    };
    match mode {
        PrivacyMode::None => json!({
            field: text,
            format!("{field}_hash"): hash_tag(text),
            format!("{field}_chars"): text.chars().count()
        }),
        PrivacyMode::Safe => json!({
            format!("{field}_hash"): hash_tag(text),
            format!("{field}_chars"): text.chars().count()
        }),
        PrivacyMode::Strict => json!({
            format!("{field}_present"): true,
            format!("{field}_chars"): text.chars().count()
        }),
    }
}

pub fn command_attrs(command: Option<&str>, mode: PrivacyMode) -> Value {
    let Some(command) = command else {
        return json!({});
    };
    let command_name = command
        .split_whitespace()
        .next()
        .map(|part| part.trim_matches(|c| c == '"' || c == '\''));
    match mode {
        PrivacyMode::None => json!({
            "command": command,
            "command_hash": hash_tag(command),
            "command_name": command_name,
        }),
        PrivacyMode::Safe => json!({
            "command_hash": hash_tag(command),
            "command_name": command_name,
        }),
        PrivacyMode::Strict => json!({
            "command_present": true,
            "command_name": command_name,
        }),
    }
}

pub fn merge_json(mut base: Value, extra: Value) -> Value {
    match (&mut base, extra) {
        (Value::Object(base), Value::Object(extra)) => {
            for (key, value) in extra {
                base.insert(key, value);
            }
            Value::Object(base.clone())
        }
        (_, extra) => extra,
    }
}
