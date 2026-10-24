//! Strings for the Rust side (tray menu and tooltip), read from the same locale file as the web UI.

use std::sync::LazyLock;

use serde_json::Value;

static EN: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../ui/src/i18n/en.json")).expect("en.json is valid JSON"));

/// Looks up a dotted key ("tray.quit"); unknown keys render as the key itself.
pub fn t(key: &str) -> String {
    key.split('.').try_fold(&*EN, |node, part| node.get(part)).and_then(Value::as_str).unwrap_or(key).to_owned()
}
