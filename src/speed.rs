//! Measurement engine for Cloudflare's public speed-test endpoints.
//!
//! Streams bytes through concurrent workers and samples throughput on a fixed
//! cadence so the UI has a smooth signal to graph, rather than one number at
//! the end of a phase.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
    /// Organisation name from `/meta` (e.g. the ISP).
    #[serde(default)]
    pub asn_org: String,
    #[serde(default)]
    pub asn: String,
    /// Measured TLS handshake time and negotiated cipher.
    #[serde(default)]
    pub tls_ms: f64,
    #[serde(default)]
    pub tls_cipher: String,
    /// Addresses `speed.cloudflare.com` resolved to - the edge nodes we
    /// actually connect to, as distinct from the user's own public IP.
    #[serde(default)]
    pub edge_ips: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LatencyStats {
    pub avg: f64,
    pub min: f64,
    pub med: f64,
    pub p25: f64,
    pub p75: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub jitter: f64,
    pub loss: f64,
    pub samples: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThroughputStats {
    pub avg: f64,
    pub min: f64,
    pub med: f64,
    pub p25: f64,
    pub p75: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    /// Total bytes transferred during the phase.
    pub bytes: u64,
    /// Steady-state duration the samples represent.
    pub duration_ms: u64,
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
    /// Boxed: a `Meta` is much larger than the other variants, and this enum
    /// travels through a channel on every frame of a test.
    Meta(Box<Meta>),
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

/// Which phases a run should exercise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestMode {
    /// Latency, then download, then upload.
    Full,
    /// Idle latency only - a "ping" equivalent.
    Latency,
    /// Latency then download.
    Download,
    /// Latency then upload.
    Upload,
}

impl TestMode {
    #[must_use]
    pub fn runs_download(self) -> bool {
        matches!(self, Self::Full | Self::Download)
    }

    #[must_use]
    pub fn runs_upload(self) -> bool {
        matches!(self, Self::Full | Self::Upload)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Latency => "LATENCY",
            Self::Download => "DOWNLOAD",
            Self::Upload => "UPLOAD",
        }
    }
}

pub struct Config {
    pub base_url: String,
    pub idle_duration: Duration,
    pub download_duration: Duration,
    pub upload_duration: Duration,
    pub concurrency: usize,
    pub sample_interval: Duration,
    /// Preferred bytes per download request. Cloudflare rate-limits large
    /// payloads (HTTP 429), so the workers start here and adapt downwards.
    pub download_chunk: usize,
    pub mode: TestMode,
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
            download_chunk: 1_000_000,
            mode: TestMode::Full,
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
        // rustls panics inside `ClientConfig::builder()` unless a process-level
        // crypto provider is installed. Doing it here means it is set up once,
        // before any TLS work, and a repeated install is simply ignored.
        let _ = rustls::crypto::ring::default_provider().install_default();

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

    fn meta_url(&self) -> String {
        format!("{}/meta", self.config.base_url)
    }

    /// Pull the richer fields from `/meta` (ASN, org, city, protocol versions).
    /// Best-effort: a failure here only costs us some detail rows.
    async fn fetch_json_meta(&self, meta: &mut Meta) {
        let Ok(resp) = self.client.get(self.meta_url()).send().await else {
            return;
        };
        let Ok(v) = resp.json::<serde_json::Value>().await else {
            return;
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        if let Some(org) = s("asOrganization") {
            meta.asn_org = org;
        }
        if let Some(n) = v.get("asn").and_then(|x| x.as_u64()) {
            meta.asn = n.to_string();
        }
        if let Some(c) = s("clientIp") {
            meta.ip = c;
        }
        if let Some(c) = s("colo") {
            meta.colo = c;
        }
        if let Some(c) = s("country") {
            meta.loc = c;
        }
        if let Some(t) = s("tlsVersion") {
            meta.tls = t;
        }
        if let Some(h) = s("httpProtocol") {
            meta.http = h;
        }
    }

    /// Time a real TLS handshake with rustls so the number reflects the actual
    /// cipher negotiation rather than being inferred from the trace header.
    async fn measure_tls(&self, host: &str, meta: &mut Meta) {
        // Best-effort probe: every early exit just skips the TLS detail.
        let Some((elapsed_ms, cipher)) = self.probe_tls(host).await else {
            return;
        };
        meta.tls_ms = elapsed_ms;
        meta.tls_cipher = cipher;
    }

    /// Returns `(handshake_ms, cipher_suite)` for a real TLS handshake.
    async fn probe_tls(&self, host: &str) -> Option<(f64, String)> {
        use std::sync::Arc as StdArc;

        let mut addrs = tokio::net::lookup_host(format!("{host}:443")).await.ok()?;
        // Prefer IPv4: it is the path the rest of the test will take, so this
        // measures the handshake the user is really getting.
        let addr = addrs.find(|a| a.is_ipv4())?;

        let tcp =
            tokio::time::timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(addr))
                .await
                .ok()?
                .ok()?;
        let _ = tcp.set_nodelay(true);

        let connector = {
            let mut roots = rustls::RootCertStore::empty();
            // `CertificateResult` reports per-platform load errors separately
            // from the certs that did load; partial success is fine here.
            let certs = rustls_native_certs::load_native_certs().certs;
            if certs.is_empty() {
                return None;
            }
            roots.add_parsable_certificates(certs);
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            tokio_rustls::TlsConnector::from(StdArc::new(config))
        };

        let name = rustls::pki_types::ServerName::try_from(host.to_string()).ok()?;

        let started = Instant::now();
        let tls = connector.connect(name, tcp).await.ok()?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let (_, session) = tls.get_ref();
        let cipher = session
            .negotiated_cipher_suite()
            .map(|c| format!("{:?}", c.suite()))
            .unwrap_or_default();
        Some((elapsed_ms, cipher))
    }

    /// Fetch connection metadata, DNS timing and a real TLS handshake timing.
    /// Every step is best-effort: a failure only costs us detail rows, never
    /// the test itself.
    pub async fn fetch_meta(&self) -> Meta {
        let started = Instant::now();
        // Resolve once and keep the addresses: this gives both the DNS timing
        // and the edge IPs we are about to connect to.
        let resolved: Vec<String> = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host("speed.cloudflare.com:443"),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(|addrs| addrs.map(|a| a.to_string()).collect())
        .unwrap_or_default();
        let dns_ms = if resolved.is_empty() {
            0.0
        } else {
            started.elapsed().as_secs_f64() * 1000.0
        };

        let mut meta = Meta {
            dns_ms,
            edge_ips: resolved,
            ..Default::default()
        };

        if let Ok(resp) = self.client.get(self.trace_url()).send().await {
            if let Ok(body) = resp.text().await {
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
            }
        }

        // `/meta` carries ASN, organisation and protocol details that the trace
        // endpoint omits, so fetch it too and let it fill any gaps.
        self.fetch_json_meta(&mut meta).await;
        self.measure_tls("speed.cloudflare.com", &mut meta).await;
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
        let _ = tx.send(Event::Meta(Box::new(meta.clone())));

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

        // Skip phases the selected mode does not exercise. A latency-only run still
        // completes, with empty throughput rather than short-circuiting.

        let (download, loaded_down) = if self.config.mode.runs_download() {
            run_phase(
                PhaseSpec {
                    client: &self.client,
                    base_url: &self.config.base_url,
                    concurrency: self.config.concurrency,
                    duration: self.config.download_duration,
                    sample_interval: self.config.sample_interval,
                    phase: Phase::Download,
                    dir: Direction::Down,
                    download_chunk: self.config.download_chunk,
                },
                tx,
            )
            .await?
        } else {
            (ThroughputStats::default(), LatencyStats::default())
        };

        let (upload, loaded_up) = if self.config.mode.runs_upload() {
            run_phase(
                PhaseSpec {
                    client: &self.client,
                    base_url: &self.config.base_url,
                    concurrency: self.config.concurrency,
                    duration: self.config.upload_duration,
                    sample_interval: self.config.sample_interval,
                    phase: Phase::Upload,
                    dir: Direction::Up,
                    download_chunk: self.config.download_chunk,
                },
                tx,
            )
            .await?
        } else {
            (ThroughputStats::default(), LatencyStats::default())
        };

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
    download_chunk: usize,
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
        download_chunk,
    } = spec;

    let _ = tx.send(Event::Phase(phase));
    let counter = Arc::new(AtomicU64::new(0));
    // Shared across workers so they collectively back off when rate limited.
    let chunk = Arc::new(AtomicUsize::new(download_chunk));
    let deadline = Instant::now() + duration;

    let mut handles = Vec::with_capacity(concurrency.max(1));
    for _ in 0..concurrency.max(1) {
        let counter = Arc::clone(&counter);
        let client = client.clone();
        let base = base_url.to_string();
        let chunk = Arc::clone(&chunk);
        handles.push(tokio::spawn(async move {
            match dir {
                Direction::Down => download_worker(client, base, counter, chunk, deadline).await,
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
    let stats = summarise_throughput(
        &samples[ramp..],
        counter.load(Ordering::Relaxed),
        duration.as_millis() as u64,
    );
    let lat = summarise_latency(&probe_samples);
    Ok((stats, lat))
}

async fn download_worker(
    client: reqwest::Client,
    base_url: String,
    counter: Arc<AtomicU64>,
    chunk: Arc<AtomicUsize>,
    deadline: Instant,
) {
    const MIN_CHUNK: usize = 100_000;
    let max_chunk = chunk.load(Ordering::Relaxed);
    let mut ok_streak = 0usize;

    while Instant::now() < deadline {
        let n = chunk.load(Ordering::Relaxed);
        let url = format!("{base_url}/__down?bytes={n}");

        let ok = match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let mut stream = resp.bytes_stream();
                while let Some(part) = stream.next().await {
                    match part {
                        Ok(bytes) => {
                            counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        }
                        Err(_) => break,
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                true
            }
            _ => false,
        };

        // Adapt the request size: shrink fast when refused, grow back slowly
        // once the endpoint is willing again.
        let current = chunk.load(Ordering::Relaxed);
        if ok {
            ok_streak += 1;
            if ok_streak >= 8 {
                ok_streak = 0;
                if current < max_chunk {
                    chunk.store((current + current / 2).min(max_chunk), Ordering::Relaxed);
                }
            }
        } else {
            ok_streak = 0;
            if current > MIN_CHUNK {
                chunk.store((current / 2).max(MIN_CHUNK), Ordering::Relaxed);
            } else {
                tokio::time::sleep(Duration::from_millis(250)).await;
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

    // Count bytes only once the server has acknowledged a complete body.
    //
    // An earlier version streamed an endless body and counted as reqwest pulled
    // chunks from it. That measures bytes handed to the kernel, not bytes
    // delivered: the counter runs ahead of the wire by whatever is sitting in
    // the socket buffer, which produced momentary readings of well over 1 Gbps
    // on a link that actually sustains ~130 Mbps. A fixed body that is counted
    // only on a 2xx response is ground truth, and bodies are kept small so
    // completions stay frequent enough for a smooth sampled curve.
    const BODY: usize = 1024 * 1024;

    let mut buf = vec![0u8; BODY];
    rand::thread_rng().fill_bytes(&mut buf);
    // `Bytes::from(Vec)` takes ownership without copying; cloning per request
    // is a refcount bump.
    let payload = Bytes::from(buf);
    let body_len = payload.len() as u64;
    let url = format!("{base_url}/__up");

    while Instant::now() < deadline {
        match client.post(&url).body(payload.clone()).send().await {
            Ok(resp) if resp.status().is_success() => {
                counter.fetch_add(body_len, Ordering::Relaxed);
            }
            Ok(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
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
        min: sorted.first().copied().unwrap_or(0.0),
        med: percentile(&sorted, 0.5),
        p25: percentile(&sorted, 0.25),
        p75: percentile(&sorted, 0.75),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        max: sorted.last().copied().unwrap_or(0.0),
        jitter: jitter(&good),
        loss,
        samples: good.len(),
    }
}

fn summarise_throughput(samples: &[f64], bytes: u64, duration_ms: u64) -> ThroughputStats {
    if samples.is_empty() {
        return ThroughputStats {
            bytes,
            duration_ms,
            ..Default::default()
        };
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ThroughputStats {
        avg: samples.iter().sum::<f64>() / samples.len() as f64,
        min: sorted.first().copied().unwrap_or(0.0),
        med: percentile(&sorted, 0.5),
        p25: percentile(&sorted, 0.25),
        p75: percentile(&sorted, 0.75),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        max: sorted.last().copied().unwrap_or(0.0),
        bytes,
        duration_ms,
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
        assert_eq!(s.samples, 2, "dropped probes must not count as samples");
    }

    #[test]
    fn latency_summary_fills_the_full_percentile_set() {
        let s = summarise_latency(&[10.0, 20.0, 30.0, 40.0]);
        assert!((s.min - 10.0).abs() < 1e-9);
        assert!((s.max - 40.0).abs() < 1e-9);
        assert!((s.p25 - 17.5).abs() < 1e-9);
        assert!((s.p75 - 32.5).abs() < 1e-9);
        assert!((s.p99 - 39.7).abs() < 1e-6);
        assert_eq!(s.samples, 4);
    }

    #[test]
    fn latency_summary_handles_all_lost_probes() {
        let s = summarise_latency(&[-1.0, -1.0]);
        assert!((s.loss - 100.0).abs() < 1e-9);
        assert_eq!(s.avg, 0.0);
    }

    #[test]
    fn throughput_summary_reports_bytes_and_window() {
        let s = summarise_throughput(&[10.0, 20.0, 30.0], 123_456, 8_000);
        assert_eq!(s.bytes, 123_456);
        assert_eq!(s.duration_ms, 8_000);
        assert!((s.avg - 20.0).abs() < 1e-9);
        assert!((s.min - 10.0).abs() < 1e-9);
        assert!((s.max - 30.0).abs() < 1e-9);
        assert_eq!(s.samples.len(), 3);
    }

    #[test]
    fn throughput_summary_keeps_totals_when_no_samples() {
        let s = summarise_throughput(&[], 999, 4_000);
        assert_eq!(s.bytes, 999, "bytes are counted even without samples");
        assert_eq!(s.samples.len(), 0);
        assert_eq!(s.avg, 0.0);
    }
}
