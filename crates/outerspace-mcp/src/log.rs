//! `logs\actions.jsonl` in the data folder: one line per tool call, the only trail of what
//! agents did (docs/PLANO.md §2.6).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const MAX_BYTES: u64 = 10 * 1024 * 1024;

pub struct ActionLog {
    file: Mutex<Option<File>>,
}

impl ActionLog {
    /// Opens (appending) the log in `dir`; past 10 MB the old file moves to `actions.1.jsonl`.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("actions.jsonl");
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            let _ = std::fs::rename(&path, dir.join("actions.1.jsonl"));
        }
        let file = OpenOptions::new().create(true).append(true).open(&path);
        if let Err(err) = &file {
            tracing::warn!(?path, %err, "action log unavailable");
        }
        ActionLog { file: Mutex::new(file.ok()) }
    }

    pub fn record(&self, tool: &str, args: Value, ok: bool, detail: &str) {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
        let line = json!({ "ts": ts, "tool": tool, "args": args, "ok": ok, "detail": detail });
        let mut file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }
}
