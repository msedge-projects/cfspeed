//! Measurement engine for Cloudflare's public speed-test endpoints.
//!
//! Streams bytes through concurrent workers and samples throughput on a fixed
//! cadence so the UI has a smooth signal to graph, rather than one number at
//! the end of a phase.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Linear-interpolated percentile. `sorted` must be ascending.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&sorted, 0.5)
}

/// Mean absolute consecutive difference — the IETF's jitter estimate.
pub fn jitter(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let sum: f64 = values.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    sum / (values.len() - 1) as f64
}

/// Coefficient of variation, as a percentage.
pub fn cv_percent(values: &[f64]) -> f64 {
    if values.len() < 3 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    (var.sqrt() / mean) * 100.0
}

/// Waveform-style bufferbloat grading, from latency added under load.
pub fn bufferbloat_grade(ms: f64) -> &'static str {
    match ms {
        m if m <= 5.0 => "A+",
        m if m <= 30.0 => "A",
        m if m <= 60.0 => "B",
        m if m <= 200.0 => "C",
        m if m <= 400.0 => "D",
        _ => "F",
    }
}

/// Stability grading, from throughput variation.
pub fn stability_grade(cv: f64) -> &'static str {
    match cv {
        c if c <= 5.0 => "A",
        c if c <= 10.0 => "B",
        c if c <= 20.0 => "C",
        c if c <= 35.0 => "D",
        _ => "F",
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Connecting,
    Idle,
    Download,
    Upload,
    Finalising,
    Done,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Connecting => "CONNECTING",
            Phase::Idle => "LATENCY",
            Phase::Download => "DOWNLOAD",
            Phase::Upload => "UPLOAD",
            Phase::Finalising => "FINALISING",
            Phase::Done => "COMPLETE",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub colo: String,
    #[serde(default)]
    pub loc: String,
    #[serde(default)]
    pub tls: String,
    #[serde(default)]
    pub warp: String,
    #[serde(default)]
    pub http: String,
    #[serde(default)]
    pub dns_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LatencyStats {
    pub avg: f64,
    pub med: f64,
    pub p25: f64,
    pub p95: f64,
    pub max: f64,
    pub jitter: f64,
    pub loss: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThroughputStats {
    pub avg: f64,
    pub med: f64,
    pub p95: f64,
    pub max: f64,
    pub samples: Vec<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub timestamp_ms: u64,
    pub meta: Meta,
    pub idle: LatencyStats,
    pub loaded_down: LatencyStats,
    pub loaded_up: LatencyStats,
    pub download: ThroughputStats,
    pub upload: ThroughputStats,
}

impl RunResult {
    /// Latency added by saturating the link — the bufferbloat signal.
    pub fn bufferbloat_ms(&self) -> f64 {
        (self.loaded_down.med - self.idle.med).max(0.0)
    }
}

#[derive(Debug)]
pub enum Event {
    Meta(Meta),
    Phase(Phase),
    /// One throughput sample, in Mbps.
    Tick {
        mbps: f64,
    },
    /// One latency sample, in milliseconds.
    Probe {
        ms: f64,
    },
    Finished(Box<RunResult>),
    Failed(String),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

pub struct Config {
    pub base_url: String,
    pub idle_duration: Duration,
    pub download_duration: Duration,
    pub upload_duration: Duration,
    pub concurrency: usize,
    pub sample_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "https://speed.cloudflare.com".to_string(),
            idle_duration: Duration::from_secs(2),
            download_duration: Duration::from_secs(10),
            upload_duration: Duration::from_secs(10),
            concurrency: 6,
            sample_interval: Duration::from_millis(200),
        }
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub struct Runner {
    config: Config,
    client: reqwest::Client,
}

impl Runner {
    pub fn new(config: Config) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(16)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { config, client })
    }

    fn down_url(&self, bytes: usize) -> String {
        format!("{}/__down?bytes={}", self.config.base_url, bytes)
    }

    fn trace_url(&self) -> String {
        format!("{}/cdn-cgi/trace", self.config.base_url)
    }

    /// Fetch connection metadata and DNS timing. Failures here are non-fatal.
    pub async fn fetch_meta(&self) -> Meta {
        let started = Instant::now();
        let dns_ms = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::net::lookup_host("speed.cloudflare.com:443")
                .await
                .ok()
        })
        .await
        .ok()
        .flatten()
        .map(|_| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

        let Ok(resp) = self.client.get(self.trace_url()).send().await else {
            return Meta {
                dns_ms,
                ..Default::default()
            };
        };
        let Ok(body) = resp.text().await else {
            return Meta {
                dns_ms,
                ..Default::default()
            };
        };

        let mut meta = Meta {
            dns_ms,
            ..Default::default()
        };
        for line in body.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k {
                "ip" => meta.ip = v.to_string(),
                "colo" => meta.colo = v.to_string(),
                "loc" => meta.loc = v.to_string(),
                "tls" => meta.tls = v.to_string(),
                "warp" => meta.warp = v.to_string(),
                "http" => meta.http = v.to_string(),
                _ => {}
            }
        }
        meta
    }

    /// Run a full test, emitting [`Event`]s as it progresses.
    pub async fn run(self, tx: mpsc::UnboundedSender<Event>) {
        if let Err(e) = self.run_inner(&tx).await {
            let _ = tx.send(Event::Failed(e.to_string()));
        }
    }

    async fn run_inner(&self, tx: &mpsc::UnboundedSender<Event>) -> Result<()> {
        let _ = tx.send(Event::Phase(Phase::Connecting));
        let meta = self.fetch_meta().await;
        let _ = tx.send(Event::Meta(meta.clone()));

        // --- Idle latency -------------------------------------------------
        let _ = tx.send(Event::Phase(Phase::Idle));
        let mut idle_samples = Vec::new();
        let idle_deadline = Instant::now() + self.config.idle_duration;
        // Aim for ~8 probes regardless of how slow each probe takes.
        let gap = (self.config.idle_duration / 8)
            .clamp(Duration::from_millis(100), Duration::from_millis(500));
        while Instant::now() < idle_deadline {
            let sent = Instant::now();
            match self
                .client
                .get(self.down_url(0))
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(resp) => {
                    let _ = resp.bytes().await;
                    let ms = sent.elapsed().as_secs_f64() * 1000.0;
                    idle_samples.push(ms);
                    let _ = tx.send(Event::Probe { ms });
                }
                Err(_) => idle_samples.push(-1.0),
            }
            tokio::time::sleep(gap).await;
        }
        let idle = summarise_latency(&idle_samples);

        let (download, loaded_down) = run_phase(
            PhaseSpec {
                client: &self.client,
                base_url: &self.config.base_url,
                concurrency: self.config.concurrency,
                duration: self.config.download_duration,
                sample_interval: self.config.sample_interval,
                phase: Phase::Download,
                dir: Direction::Down,
            },
            tx,
        )
        .await?;
        let (upload, loaded_up) = run_phase(
            PhaseSpec {
                client: &self.client,
                base_url: &self.config.base_url,
                concurrency: self.config.concurrency,
                duration: self.config.upload_duration,
                sample_interval: self.config.sample_interval,
                phase: Phase::Upload,
                dir: Direction::Up,
            },
            tx,
        )
        .await?;

        let _ = tx.send(Event::Phase(Phase::Finalising));
        let result = RunResult {
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            meta,
            idle,
            loaded_down,
            loaded_up,
            download,
            upload,
        };
        let _ = tx.send(Event::Phase(Phase::Done));
        let _ = tx.send(Event::Finished(Box::new(result)));
        Ok(())
    }
}

// Placeholder removed — throughput stats are returned by value below.

#[derive(Clone, Copy)]
enum Direction {
    Down,
    Up,
}

/// Parameters for a single throughput phase.
struct PhaseSpec<'a> {
    client: &'a reqwest::Client,
    base_url: &'a str,
    concurrency: usize,
    duration: Duration,
    sample_interval: Duration,
    phase: Phase,
    dir: Direction,
}

/// Run one throughput phase, emitting `Tick`s while it runs.
///
/// Returns the steady-state throughput summary and the latency observed while
/// the link was saturated.
async fn run_phase(
    spec: PhaseSpec<'_>,
    tx: &mpsc::UnboundedSender<Event>,
) -> Result<(ThroughputStats, LatencyStats)> {
    let PhaseSpec {
        client,
        base_url,
        concurrency,
        duration,
        sample_interval,
        phase,
        dir,
    } = spec;

    let _ = tx.send(Event::Phase(phase));
    let counter = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + duration;

    let mut handles = Vec::with_capacity(concurrency.max(1));
    for _ in 0..concurrency.max(1) {
        let counter = Arc::clone(&counter);
        let client = client.clone();
        let base = base_url.to_string();
        handles.push(tokio::spawn(async move {
            match dir {
                Direction::Down => download_worker(client, &base, counter, deadline).await,
                Direction::Up => upload_worker(client, &base, counter, deadline).await,
            }
        }));
    }

    // Sampler: convert the cumulative byte counter into instantaneous Mbps.
    let mut samples: Vec<f64> = Vec::new();
    let mut probe_samples: Vec<f64> = Vec::new();
    let mut last_bytes = 0u64;
    let mut last = Instant::now();
    loop {
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(sample_interval).await;
        if Instant::now() >= deadline {
            break;
        }

        let now = Instant::now();
        let bytes = counter.load(Ordering::Relaxed);
        let dt = now.duration_since(last).as_secs_f64();
        if dt > 0.0 && bytes >= last_bytes {
            let mbps = (bytes - last_bytes) as f64 * 8.0 / dt / 1.0e6;
            samples.push(mbps);
            let _ = tx.send(Event::Tick { mbps });
        }
        last_bytes = bytes;
        last = now;

        // Interleaved latency probe measures latency while the link is loaded.
        let sent = Instant::now();
        if let Ok(resp) = client
            .get(format!("{base_url}/__down?bytes=0"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            let _ = resp.bytes().await;
            let ms = sent.elapsed().as_secs_f64() * 1000.0;
            probe_samples.push(ms);
            let _ = tx.send(Event::Probe { ms });
        }
    }

    for h in handles {
        h.abort();
    }

    // Drop the ramp-up (TCP slow start) before summarising.
    let ramp = ((samples.len() as f64 * 0.2) as usize)
        .max(1)
        .min(samples.len());
    let stats = summarise_throughput(&samples[ramp..]);
    let lat = summarise_latency(&probe_samples);
    Ok((stats, lat))
}

async fn download_worker(
    client: reqwest::Client,
    base_url: &str,
    counter: Arc<AtomicU64>,
    deadline: Instant,
) {
    const CHUNK: usize = 25_000_000;
    let url = format!("{base_url}/__down?bytes={CHUNK}");
    while Instant::now() < deadline {
        let Ok(resp) = client.get(&url).send().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let Ok(resp) = resp.error_for_status() else {
            continue;
        };
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
    }
}

async fn upload_worker(
    client: reqwest::Client,
    base_url: &str,
    counter: Arc<AtomicU64>,
    deadline: Instant,
) {
    use bytes::Bytes;
    use rand::RngCore;
    use std::sync::atomic::AtomicUsize;

    // Each request sends a *finite* body: `send()` resolves on response headers,
    // so an endless stream would be abandoned mid-flight and restarted,
    // crediting bytes the server never received.
    //
    // The body is still streamed in chunks rather than handed over as one
    // blob, so the byte counter advances at roughly socket pace and the
    // sampled curve stays smooth instead of stepping by 25MB per request.
    const CHUNK: usize = 250 * 1024;
    const CHUNKS: usize = 100; // 25MB per request

    let payload: Bytes = {
        let mut v = vec![0u8; CHUNK];
        rand::thread_rng().fill_bytes(&mut v);
        // `Bytes::from(Vec)` takes ownership without copying; cloning per
        // chunk is just a refcount bump.
        Bytes::from(v)
    };
    let chunk_len = payload.len() as u64;
    let url = format!("{base_url}/__up");

    while Instant::now() < deadline {
        let remaining = Arc::new(AtomicUsize::new(CHUNKS));
        let shared = payload.clone();
        let shared_counter = Arc::clone(&counter);
        let stream = futures::stream::unfold((), move |()| {
            let payload = shared.clone();
            let counter = Arc::clone(&shared_counter);
            let remaining = Arc::clone(&remaining);
            async move {
                if remaining.fetch_sub(1, Ordering::SeqCst) == 0 {
                    return None;
                }
                counter.fetch_add(chunk_len, Ordering::Relaxed);
                Some((Ok::<Bytes, std::io::Error>(payload), ()))
            }
        });
        let body = reqwest::Body::wrap_stream(stream);
        if let Err(e) = client.post(&url).body(body).send().await {
            let _ = e;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn summarise_latency(samples: &[f64]) -> LatencyStats {
    let sent = samples.len();
    let good: Vec<f64> = samples.iter().copied().filter(|v| *v >= 0.0).collect();
    let loss = if sent == 0 {
        0.0
    } else {
        ((sent - good.len()) as f64 / sent as f64) * 100.0
    };
    if good.is_empty() {
        return LatencyStats {
            loss,
            ..Default::default()
        };
    }
    let mut sorted = good.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    LatencyStats {
        avg: good.iter().sum::<f64>() / good.len() as f64,
        med: percentile(&sorted, 0.5),
        p25: percentile(&sorted, 0.25),
        p95: percentile(&sorted, 0.95),
        max: sorted.last().copied().unwrap_or(0.0),
        jitter: jitter(&good),
        loss,
    }
}

fn summarise_throughput(samples: &[f64]) -> ThroughputStats {
    if samples.is_empty() {
        return ThroughputStats::default();
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ThroughputStats {
        avg: samples.iter().sum::<f64>() / samples.len() as f64,
        med: percentile(&sorted, 0.5),
        p95: percentile(&sorted, 0.95),
        max: sorted.last().copied().unwrap_or(0.0),
        samples: samples.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_interpolates() {
        let s = [1.0, 2.0, 3.0, 4.0];
        assert!((percentile(&s, 0.0) - 1.0).abs() < 1e-9);
        assert!((percentile(&s, 0.5) - 2.5).abs() < 1e-9);
        assert!((percentile(&s, 1.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn percentile_handles_empty() {
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn median_of_odd_count() {
        assert!((median(&[5.0, 1.0, 3.0]) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn jitter_measures_consecutive_delta() {
        assert!((jitter(&[10.0, 20.0, 10.0]) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn cv_is_zero_for_constant_signal() {
        assert!(cv_percent(&[100.0, 100.0, 100.0, 100.0]).abs() < 1e-9);
    }

    #[test]
    fn cv_grows_with_spread() {
        let steady = cv_percent(&[100.0, 101.0, 99.0, 100.0]);
        let wild = cv_percent(&[0.0, 200.0, 100.0, 300.0]);
        assert!(wild > steady);
    }

    #[test]
    fn grades_follow_thresholds() {
        assert_eq!(bufferbloat_grade(3.0), "A+");
        assert_eq!(bufferbloat_grade(45.0), "B");
        assert_eq!(bufferbloat_grade(9999.0), "F");
        assert_eq!(stability_grade(1.0), "A");
        assert_eq!(stability_grade(100.0), "F");
    }

    #[test]
    fn latency_summary_counts_loss() {
        let s = summarise_latency(&[10.0, -1.0, 20.0, -1.0]);
        assert!((s.loss - 50.0).abs() < 1e-9);
        assert!((s.avg - 15.0).abs() < 1e-9);
    }
}
