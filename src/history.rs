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

/// Write an export, preferring the working directory so the file lands where
/// the user is looking, but falling back to the data directory when that is
/// not writable. Running `cfspeed` from a read-only location (a Program Files
/// install, a locked share) would otherwise fail with nothing to show.
fn write_export(name: &str, body: &str) -> Result<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let path = cwd.join(name);
        if fs::write(&path, body).is_ok() {
            return Ok(path);
        }
    }

    let dir = data_dir();
    fs::create_dir_all(&dir).context("creating export directory")?;
    let path = dir.join(name);
    fs::write(&path, body).with_context(|| format!("writing {name}"))?;
    Ok(path)
}

/// Write every saved run to `cfspeed-history.json`.
pub fn export_json(runs: &[RunResult]) -> Result<PathBuf> {
    let payload = serde_json::to_string_pretty(runs)?;
    write_export("cfspeed-history.json", &payload)
}

/// Write every saved run to `cfspeed-history.csv`.
pub fn export_csv(runs: &[RunResult]) -> Result<PathBuf> {
    let header = [
        "timestamp_utc",
        "colo",
        "ip",
        "download_avg_mbps",
        "upload_avg_mbps",
        "download_p95_mbps",
        "upload_p95_mbps",
        "download_bytes",
        "upload_bytes",
        "idle_median_ms",
        "loaded_download_median_ms",
        "jitter_ms",
        "loss_pct",
        "bufferbloat_ms",
        "stability_cv_pct",
    ];

    let mut out = String::new();
    out.push_str(&header.join(","));
    out.push('\n');

    for r in runs {
        let dl_cv = cv(&r.download.samples);
        let ul_cv = cv(&r.upload.samples);
        let row = [
            r.timestamp_ms.to_string(),
            csv_field(&r.meta.colo),
            csv_field(&r.meta.ip),
            f2(r.download.avg),
            f2(r.upload.avg),
            f2(r.download.p95),
            f2(r.upload.p95),
            r.download.bytes.to_string(),
            r.upload.bytes.to_string(),
            f2(r.idle.med),
            f2(r.loaded_down.med),
            f2(r.idle.jitter),
            f2(r.idle.loss),
            f2(r.bufferbloat_ms()),
            f2(dl_cv.max(ul_cv)),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }

    write_export("cfspeed-history.csv", &out)
}

fn cv(samples: &[f64]) -> f64 {
    crate::speed::cv_percent(samples)
}

fn f2(v: f64) -> String {
    format!("{v:.2}")
}

/// Quote a value if it contains a comma, quote or newline.
fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// Delete the saved history file and return its path.
pub fn clear() -> Result<PathBuf> {
    let path = history_path();
    if path.exists() {
        fs::remove_file(&path).context("removing history file")?;
    }
    Ok(path)
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
