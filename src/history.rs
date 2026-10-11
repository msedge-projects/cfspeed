//! Persistence for completed runs.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::speed::RunResult;

const MAX_STORED: usize = 200;

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    runs: Vec<RunResult>,
}

/// `%LOCALAPPDATA%\cfspeed` on Windows, `~/.local/share/cfspeed` elsewhere.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(dir).join("cfspeed");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/cfspeed");
    }
    PathBuf::from(".cfspeed")
}

fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

/// Load saved runs, newest last. A missing or corrupt file yields an empty
/// history rather than an error - history is a convenience, not a requirement.
#[must_use]
pub fn load() -> Vec<RunResult> {
    let path = history_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<HistoryFile>(&raw)
        .map(|f| f.runs)
        .unwrap_or_default()
}

/// Persist the full set of runs, trimming to `MAX_STORED`.
pub fn save(runs: &[RunResult]) -> Result<()> {
    let mut trimmed = runs.to_vec();
    if trimmed.len() > MAX_STORED {
        let excess = trimmed.len() - MAX_STORED;
        trimmed.drain(..excess);
    }
    let dir = data_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let payload = serde_json::to_string_pretty(&HistoryFile { runs: trimmed })?;
    fs::write(history_path(), payload).context("writing history file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_location_still_resolves() {
        // data_dir must always return something usable, never an empty path.
        assert!(!data_dir().as_os_str().is_empty());
    }

    #[test]
    fn history_file_roundtrips_empty_list() {
        let f = HistoryFile { runs: Vec::new() };
        let json = serde_json::to_string(&f).expect("serialise");
        let back: HistoryFile = serde_json::from_str(&json).expect("deserialise");
        assert!(back.runs.is_empty());
    }
}
