// Range bar forensic telemetry — bit-correct NDJSON artifacts
// GitHub Issue: https://github.com/terrylica/flowsurface/issues/telemetry
//
// Activation: compile with `--features telemetry` + runtime `FLOWSURFACE_TELEMETRY=1`.
// Zero-cost when the feature is absent (all call sites gated by #[cfg(feature = "telemetry")]).

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::thread;

use chrono::Utc;
use serde::Serialize;

use crate::aggr::ticks::RangeBarMicrostructure;
use exchange::Kline;

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static WRITER: OnceLock<TelemetryWriter> = OnceLock::new();

/// Initialize the telemetry writer. No-op if already initialized or env var absent.
/// Call once at startup (e.g. after logger setup).
pub fn init() {
    if std::env::var("FLOWSURFACE_TELEMETRY").as_deref() != Ok("1") {
        return;
    }
    let _ = WRITER.get_or_init(|| match TelemetryWriter::new() {
        Ok(w) => {
            log::info!("[telemetry] writer initialized, dir={}", w.dir.display());
            w
        }
        Err(e) => {
            log::error!("[telemetry] failed to initialize: {e}");
            panic!("telemetry init failed: {e}");
        }
    });
}

/// Emit a telemetry event. No-op when writer is not initialized.
pub fn emit(event: TelemetryEvent) {
    if let Some(writer) = WRITER.get() {
        // Pre-serialize on caller thread (cheap for small events)
        if let Ok(mut buf) = serde_json::to_vec(&event) {
            buf.push(b'\n');
            let _ = writer.sender.send(TelemetryMessage::Line(buf));
        }
    }
}

/// Check whether telemetry is active (writer initialized).
pub fn is_active() -> bool {
    WRITER.get().is_some()
}

// ---------------------------------------------------------------------------
// Background writer
// ---------------------------------------------------------------------------

const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB hard cap

enum TelemetryMessage {
    Line(Vec<u8>),
    Shutdown,
}

struct TelemetryWriter {
    sender: mpsc::Sender<TelemetryMessage>,
    dir: PathBuf,
    _thread: thread::JoinHandle<()>,
}

impl TelemetryWriter {
    fn new() -> io::Result<Self> {
        let dir = crate::data_path(Some("telemetry"));
        fs::create_dir_all(&dir)?;

        // Rotate old files at startup
        if let Err(e) = rotate_old_files(&dir) {
            log::warn!("[telemetry] rotation failed: {e}");
        }

        let (sender, receiver) = mpsc::channel::<TelemetryMessage>();
        let dir_clone = dir.clone();

        let thread = thread::Builder::new()
            .name("telemetry-writer".to_string())
            .spawn(move || {
                writer_loop(receiver, &dir_clone);
            })?;

        Ok(TelemetryWriter {
            sender,
            dir,
            _thread: thread,
        })
    }
}

impl Drop for TelemetryWriter {
    fn drop(&mut self) {
        let _ = self.sender.send(TelemetryMessage::Shutdown);
    }
}

fn writer_loop(receiver: mpsc::Receiver<TelemetryMessage>, dir: &Path) {
    let path = daily_path(dir);
    let file = match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[telemetry] cannot open {}: {e}", path.display());
            return;
        }
    };
    let mut current_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut writer = BufWriter::new(file);

    loop {
        match receiver.recv() {
            Ok(TelemetryMessage::Line(data)) => {
                let len = data.len() as u64;
                if current_size + len > MAX_FILE_SIZE {
                    eprintln!(
                        "[telemetry] file size cap reached ({MAX_FILE_SIZE} bytes), stopping"
                    );
                    break;
                }
                if writer.write_all(&data).is_ok() {
                    current_size += len;
                    // Flush periodically (BufWriter handles batching)
                }
            }
            Ok(TelemetryMessage::Shutdown) | Err(_) => {
                let _ = writer.flush();
                break;
            }
        }
    }
}

fn daily_path(dir: &Path) -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d");
    dir.join(format!("rb-{date}.ndjson"))
}

/// Delete .ndjson files older than 7 days.
fn rotate_old_files(dir: &Path) -> io::Result<()> {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "ndjson")
            && let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
            && modified < cutoff
        {
            log::info!("[telemetry] removing old file: {}", path.display());
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Top-level telemetry event, serialized as tagged NDJSON.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum TelemetryEvent {
    /// Emitted once at startup
    SessionStart {
        ts_ms: u64,
        build_version: &'static str,
    },

    /// ClickHouse poll delivers a bar (before reconciliation)
    ChPollBar {
        ts_ms: u64,
        symbol: String,
        threshold_dbps: u32,
        kline: KlineSnapshot,
        raw_f64: Option<ChKlineRaw>,
    },

    /// Summary after initial historical fetch
    ChInitialFetch {
        ts_ms: u64,
        symbol: String,
        threshold_dbps: u32,
        bar_count: usize,
        oldest_ts: u64,
        newest_ts: u64,
        micro_count: usize,
    },

    /// opendeviationbar-core processor completes a bar
    RbpBarComplete {
        ts_ms: u64,
        symbol: String,
        threshold_dbps: u32,
        kline: KlineSnapshot,
        trade_count: u32,
        ofi: f32,
        trade_intensity: f32,
        completed_bar_index: u32,
    },

    /// replace_or_append_kline() decision point
    Reconcile {
        ts_ms: u64,
        action: ReconcileAction,
        incoming: KlineSnapshot,
        existing_last: Option<KlineSnapshot>,
        micro_before: Option<RangeBarMicrostructure>,
    },

    /// Microstructure cleared during REPLACE reconciliation
    MicroLoss {
        ts_ms: u64,
        bar_time_ms: u64,
        micro_before: RangeBarMicrostructure,
    },

    /// Periodic state dump (every 30s)
    ChartSnapshot {
        ts_ms: u64,
        symbol: String,
        threshold_dbps: u32,
        total_bars: usize,
        visible_bars: usize,
        newest_bar_ts: u64,
        oldest_bar_ts: u64,
        forming_bar_ts: Option<u64>,
        rbp_completed_count: u32,
    },

    /// Chart opened/reopened
    ChartOpen {
        ts_ms: u64,
        symbol: String,
        threshold_dbps: u32,
        bar_count: usize,
        micro_coverage: usize,
    },

    /// 1-in-N WebSocket trade sample
    WsTradeSample {
        ts_ms: u64,
        trade_time_ms: u64,
        price_units: i64,
        price_f32: f32,
        qty_units: i64,
        is_sell: bool,
        seq_id: i64,
    },
}

// ---------------------------------------------------------------------------
// Sub-types for bit-correct capture
// ---------------------------------------------------------------------------

/// Captures a Kline in both fixed-point (i64 units) and rendered (f32) representations.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct KlineSnapshot {
    pub time_ms: u64,
    pub open_units: i64,
    pub close_units: i64,
    pub high_units: i64,
    pub low_units: i64,
    pub open_f32: f32,
    pub close_f32: f32,
    pub high_f32: f32,
    pub low_f32: f32,
    pub buy_vol_units: i64,
    pub sell_vol_units: i64,
}

impl KlineSnapshot {
    pub fn from_kline(k: &Kline) -> Self {
        let (buy_vol, sell_vol) = match k.volume {
            exchange::Volume::BuySell(b, s) => (b.units, s.units),
            exchange::Volume::TotalOnly(t) => (t.units, 0),
        };
        KlineSnapshot {
            time_ms: k.time,
            open_units: k.open.units,
            close_units: k.close.units,
            high_units: k.high.units,
            low_units: k.low.units,
            open_f32: k.open.to_f32(),
            close_f32: k.close.to_f32(),
            high_f32: k.high.to_f32(),
            low_f32: k.low.to_f32(),
            buy_vol_units: buy_vol,
            sell_vol_units: sell_vol,
        }
    }
}

/// Raw f64 values as read from ClickHouse (before f32 conversion).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChKlineRaw {
    pub open_f64: f64,
    pub high_f64: f64,
    pub low_f64: f64,
    pub close_f64: f64,
    pub buy_volume_f64: f64,
    pub sell_volume_f64: f64,
}

impl ChKlineRaw {
    pub fn from_array(a: [f64; 6]) -> Self {
        ChKlineRaw {
            open_f64: a[0],
            high_f64: a[1],
            low_f64: a[2],
            close_f64: a[3],
            buy_volume_f64: a[4],
            sell_volume_f64: a[5],
        }
    }
}

/// Classification of the reconciliation action taken.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum ReconcileAction {
    /// Same timestamp — ClickHouse bar replaces locally-built bar
    Replace,
    /// Newer timestamp — appended after last bar
    Append,
    /// Older timestamp — silently dropped (stale data)
    Drop,
    /// No existing bars — first bar appended to empty series
    AppendEmpty,
}

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
