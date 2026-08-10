//! End-to-end scaling test at 1M / 10M / 100M scale.
//!
//! Runs the full cloud-sync lifecycle — **load** (`append_all` + `flush`),
//! **recover** (drop + reopen), **drain** (`read_from` + `delete_acked`) — and
//! reports wall-clock throughput for each phase. Verifies sequence integrity
//! (gap-free, in-order, exactly `count` items seen) at the end.
//!
//! This is the workload class [`docs/PERFORMANCE.md`](../docs/PERFORMANCE.md)
//! explicitly says is **NOT** covered by the criterion micro-benchmarks: a
//! single long run, real disk, real segment counts. Run it on the target
//! deployment machine for numbers that reflect production, not tmpfs.
//!
//! # Usage
//!
//! ```text
//! cargo run --release --example scaling -- [OPTIONS] [count] [batch_size] [compression] [payload_mult] [payload_kind]
//!
//! # Defaults: 1M items, batch 5000, zstd-3, 64B uniform payload, tmpfs
//! cargo run --release --example scaling
//!
//! # 100M on real disk (NOT tmpfs)
//! cargo run --release --example scaling -- --dir /var/tmp/sb-bench 100000000 10000 1
//!
//! # 1M with encryption (requires --features encryption)
//! # cargo run --release --example scaling --features encryption -- --encrypted 1000000
//!
//! # 1M, 10x payload, semi-compressible text
//! cargo run --release --example scaling -- 1000000 5000 3 10 text
//!
//! # 1M, 10x, pseudo-random hex (worst-case compression)
//! # cargo run --release --example scaling -- 1000000 5000 3 10 random
//! ```
//!
//! # Options
//!
//! | Flag           | Default     | What it does                                              |
//! | -------------- | ----------- | -------------------------------------------------------- |
//! | `--dir DIR`    | tempdir     | Use a real directory instead of tmpfs. Tests real disk.  |
//! | `--encrypted`  | off         | Enable XChaCha20-Poly1305 encryption (needs `encryption`). |
//! | `--keep`       | off         | Don't delete `--dir` after the run (for inspection).      |
//!
//! # Payload kinds
//!
//! The `payload_kind` arg selects the entropy of the payload string, which
//! dominates the compression ratio and therefore the on-disk footprint:
//!
//! | kind       | entropy | typical zstd ratio | models                                   |
//!| ----------- | ------- | ------------------ | ---------------------------------------- |
//!| `uniform`   | lowest  | 50-600x            | uniform fill byte — best-case baseline    |
//!| `text`      | medium  | 3-6x               | log-line-like text with varied values     |
//!| `json`      | medium  | 3-5x               | semi-structured JSON with varying fields  |
//!| `random`    | highest | ~1.1x              | pseudo-random hex — worst-case baseline   |
//!
//! `uniform` answers "what's the ceiling?"; `random` answers "what's the
//! floor?"; `text` and `json` model real-world telemetry. All payloads are
//! deterministic (seeded by item id), so runs are reproducible.
//!
//! # Latency
//!
//! In addition to throughput, the load and drain phases record per-batch
//! latencies and report **p50 / p95 / p99** percentiles. These surface tail
//! latency from the inline flush on threshold-crossing appends and from
//! disk I/O on the drain path.
//!
//! # Disk estimate
//!
//! The `payload_mult` argument multiplies the base 64-byte payload, so
//! uncompressed item size is `17 + 64 * payload_mult` bytes. At the default
//! (`payload_mult=1`) that's 81 B/item. The compressed size depends heavily on
//! `payload_kind`: uniform compresses 50-600x, real-world text/JSON compresses
//! 2-5x, random compresses ~1.1x. Check `df` before launching large
//! `payload_mult` x large `count` with low-compressibility payloads. The
//! `Throughput` durability policy (no per-flush fsync) is used because this
//! models the cloud-sync deployment where the cloud is the durable layer — edit
//! the constant below to test `Maximal`/`Segment`.

// Example/demo code: unwrap/expect, `as` conversions, and counter
// arithmetic are idiomatic in examples for clarity.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
#[cfg(feature = "encryption")]
use segment_buffer::{SegmentCipher, XChaCha20Poly1305Cipher};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Fixed-size event: the payload length is `64 * payload_mult` bytes, filled
/// per [`PayloadKind`]. Each item carries enough structure (id, timestamp, kind)
/// to verify ordering after the round-trip.
#[derive(Serialize, Deserialize, Clone)]
struct Event {
    id: u64,
    timestamp_ms: u64,
    kind: u8,
    payload: String,
}

/// Payload entropy profile. See the file-level docs for the compression-ratio
/// range of each variant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PayloadKind {
    /// Uniform fill byte — maximum compression, best-case throughput baseline.
    Uniform,
    /// Log-line-like text drawn from a small vocabulary with varied numbers.
    /// Models server/agent telemetry; compresses 3-6x.
    Text,
    /// Semi-structured JSON objects with varying field values. Models event
    /// pipelines; compresses 3-5x.
    Json,
    /// Pseudo-random hex string — near-incompressible. Worst-case baseline
    /// for disk footprint and I/O.
    Random,
}

impl PayloadKind {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "uniform" | "u" => Ok(Self::Uniform),
            "text" | "t" => Ok(Self::Text),
            "json" | "j" => Ok(Self::Json),
            "random" | "r" => Ok(Self::Random),
            other => Err(format!(
                "unknown payload_kind '{other}' (expected: uniform|text|json|random)"
            )),
        }
    }
}

impl std::fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uniform => write!(f, "uniform"),
            Self::Text => write!(f, "text"),
            Self::Json => write!(f, "json"),
            Self::Random => write!(f, "random"),
        }
    }
}

/// Deterministic PRNG (`SplitMix64`) so every run with the same item ids
/// produces byte-identical payloads — reproducible benchmarks without a `rand`
/// dependency. Seeded per-item from the event id.
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        // SplitMix64 produces a degenerate first value from seed 0; mixing in
        // a constant gives item 0 a well-distributed payload too.
        Self(seed.wrapping_add(0x9E3779B9_7F4A7C15))
    }
    const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B9_7F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF5847_6D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049B_B133111EB);
        z ^ (z >> 31)
    }
    fn pick<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
        let i = (self.next_u64() % slice.len() as u64) as usize;
        &slice[i]
    }
}

/// Small vocabulary for the `Text` and `Json` payload kinds. Chosen so the
/// resulting payloads compress like real English/log telemetry (3-6x) rather
/// than the 100-600x of a uniform fill.
const WORDS: &[&str] = &[
    "event",
    "user",
    "action",
    "status",
    "request",
    "response",
    "error",
    "warn",
    "info",
    "debug",
    "trace",
    "worker",
    "handler",
    "service",
    "module",
    "session",
    "token",
    "cache",
    "queue",
    "batch",
    "timeout",
    "retry",
    "connect",
    "close",
    "auth",
    "login",
    "logout",
    "create",
    "update",
    "delete",
    "read",
    "write",
    "flush",
    "sync",
    "commit",
    "abort",
    "start",
    "stop",
    "pause",
    "resume",
    "init",
    "shutdown",
    "health",
    "metric",
    "counter",
    "gauge",
    "span",
    "ctx",
    "node",
    "shard",
    "partition",
    "offset",
    "latency",
    "throughput",
    "memory",
    "cpu",
    "disk",
    "network",
    "db",
    "redis",
    "kafka",
    "grpc",
    "http",
];

const LEVELS: &[&str] = &["INFO", "WARN", "ERROR", "DEBUG"];

/// Build a payload string of approximately `target_len` bytes for event `id`
/// under entropy profile `kind`. All variants are deterministic in `id` and
/// `target_len`.
fn make_payload(id: u64, kind: PayloadKind, target_len: usize) -> String {
    if target_len == 0 {
        return String::new();
    }
    match kind {
        PayloadKind::Uniform => "x".repeat(target_len),
        PayloadKind::Text => {
            let mut rng = SplitMix64::new(id);
            let mut s = String::with_capacity(target_len + 64);
            while s.len() < target_len {
                // 2026-07-21T12:00:00.000Z INFO worker=12 action=flush status=ok n=123456789
                let ts = 1_700_000_000 + id;
                let level = rng.pick(LEVELS);
                let w1 = rng.pick(WORDS);
                let w2 = rng.pick(WORDS);
                let worker = rng.next_u64() % 64;
                let n = rng.next_u64();
                s.push_str(&format!("{ts} {level} worker={worker} {w1}={w2} n={n} "));
            }
            s.truncate(target_len);
            s
        }
        PayloadKind::Json => {
            let mut rng = SplitMix64::new(id);
            let mut s = String::from('[');
            while s.len() < target_len {
                let word = rng.pick(WORDS);
                let level = rng.pick(LEVELS);
                let num = rng.next_u64();
                let f = (rng.next_u64() as f64) / (u64::MAX as f64) * 1000.0;
                if !s.ends_with('[') {
                    s.push(',');
                }
                s.push_str(&format!(
                    r#"{{"id":{id},"lvl":"{level}","k":"{word}","v":{num},"f":{f:.3}}}"#
                ));
            }
            s.push(']');
            s.truncate(target_len);
            s
        }
        PayloadKind::Random => {
            let mut rng = SplitMix64::new(id);
            let mut s = String::with_capacity(target_len + 16);
            while s.len() < target_len {
                let n = rng.next_u64();
                // hex encode → ASCII-safe, near-uniform entropy, incompressible
                s.push_str(&format!("{n:016x}"));
            }
            s.truncate(target_len);
            s
        }
    }
}

/// The durability policy under test. `Throughput` (no fsync) models the
/// cloud-sync deployment. Change to `Maximal` or `Segment` to measure the
/// fsync-bound regime.
const DURABILITY: DurabilityPolicy = DurabilityPolicy::Throughput;

/// Simple latency histogram for per-batch timings.
/// Stores durations in microseconds and computes nearest-rank percentiles.
struct LatencyHistogram {
    samples: Vec<f64>, // microseconds
}

impl LatencyHistogram {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    fn record(&mut self, elapsed: Duration) {
        self.samples.push(elapsed.as_secs_f64() * 1_000_000.0);
    }

    fn percentile(&self, p: f64) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64).ceil() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / self.samples.len() as f64
    }

    fn count(&self) -> usize {
        self.samples.len()
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Parse args: flags first, then positional ---
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir_override: Option<String> = None;
    let mut encrypted = false;
    let mut keep_dir = false;
    let mut positional: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                dir_override = args.get(i).cloned();
            }
            "--encrypted" => encrypted = true,
            "--keep" => keep_dir = true,
            other if !other.starts_with("--") => positional.push(other),
            other => {
                eprintln!("unknown flag: {other}");
                eprintln!("usage: scaling [--dir DIR] [--encrypted] [--keep] [count] [batch] [compression] [payload_mult] [payload_kind]");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let count: u64 = positional
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let batch: usize = positional
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000);
    let compression: i32 = positional.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let payload_mult: usize = positional.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let payload_kind = match positional.get(4) {
        Some(s) => PayloadKind::parse(s)?,
        None => PayloadKind::Uniform,
    };

    let batch = batch.max(1);
    let payload_mult = payload_mult.max(1);
    let payload_len = 64 * payload_mult;
    let bytes_per_item: u64 = 8 + 8 + 1 + payload_len as u64;

    // --- Directory: tempdir (tmpfs) or --dir override (real disk) ---
    let (dir, _tmp_keep) = match &dir_override {
        Some(path) => {
            std::fs::create_dir_all(path)?;
            (std::path::PathBuf::from(path), None)
        }
        None => {
            let tmp = tempfile::tempdir()?;
            let dir = tmp.path().to_path_buf();
            (dir, Some(tmp))
        }
    };

    println!("=== segment-buffer scaling test ===");
    println!(
        "count: {count} | batch: {batch} | compression: zstd-{compression} | durability: {DURABILITY:?}"
    );
    println!(
        "payload: {payload_kind} {payload_len} B/item ({payload_mult}x base-64) | uncompressed: {bytes_per_item} B/item"
    );
    let encrypted_label = if encrypted {
        "yes (XChaCha20-Poly1305)"
    } else {
        "no"
    };
    println!("encrypted: {encrypted_label}");
    println!("dir: {}", dir.display());
    let is_tmpfs = dir_override.is_none();
    println!("disk: {}", if is_tmpfs { "tmpfs" } else { "real disk" });
    println!();

    #[allow(unused_mut)]
    let mut config_builder = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(u64::MAX)
        .compression_level(compression)
        .durability(DURABILITY);

    #[cfg(feature = "encryption")]
    if encrypted {
        let key = [0x42u8; 32];
        let cipher: std::sync::Arc<dyn SegmentCipher + Send + Sync> =
            std::sync::Arc::new(XChaCha20Poly1305Cipher::new(&key));
        config_builder = config_builder.cipher(cipher);
    }

    #[cfg(not(feature = "encryption"))]
    if encrypted {
        eprintln!("--encrypted requires --features encryption");
        std::process::exit(2);
    }

    let config = config_builder.build();

    // ------------------------------------------------------------------
    // Phase 1: LOAD — append_all in batches + flush per batch.
    //
    // Payloads are generated OUTSIDE the timed window so the load throughput
    // reflects only the buffer (CBOR + zstd + I/O), not the cost of building
    // the payload strings. Wall time (including generation) is shown in the
    // heartbeat for operators who care about the full producer cost.
    // ------------------------------------------------------------------
    println!("--- phase 1: load (append_all + flush per batch) ---");
    let buf = SegmentBuffer::<Event>::open(&dir, config.clone())?;
    let wall_start = Instant::now();
    let mut load_elapsed = std::time::Duration::ZERO;
    let mut load_latencies = LatencyHistogram::new();
    let mut id = 0u64;
    let heartbeat = (count / 10).max(1);
    let mut next_heartbeat = heartbeat;
    while id < count {
        let take = std::cmp::min(batch as u64, count - id) as usize;
        // Untimed: payload generation (format!, String alloc) is producer cost.
        let items: Vec<Event> = (0..take)
            .map(|i| {
                let eid = id + i as u64;
                Event {
                    id: eid,
                    timestamp_ms: eid,
                    kind: (eid % 4) as u8,
                    payload: make_payload(eid, payload_kind, payload_len),
                }
            })
            .collect();
        // Timed: only the buffer operation.
        let t = Instant::now();
        let last = buf.append_all(items)?;
        buf.flush()?;
        let batch_elapsed = t.elapsed();
        load_elapsed += batch_elapsed;
        load_latencies.record(batch_elapsed);
        id = last + 1;
        if id >= next_heartbeat {
            eprintln!(
                "  ... {id}/{count} items flushed ({:.1}s wall, {:.2}s buffer)",
                wall_start.elapsed().as_secs_f64(),
                load_elapsed.as_secs_f64()
            );
            next_heartbeat += heartbeat;
        }
    }
    let peak_disk = buf.stats().approx_disk_bytes;
    assert_eq!(
        buf.latest_sequence(),
        count.saturating_sub(1),
        "load phase: latest_sequence should be count-1"
    );
    drop(buf); // release the single-process lock so we can reopen

    let load_secs = load_elapsed.as_secs_f64();
    let wall_secs = wall_start.elapsed().as_secs_f64();
    let load_ips = count as f64 / load_secs;
    println!("items/sec:  {load_ips:.0}");
    println!(
        "MiB/s:      {:.1} (uncompressed, est. {bytes_per_item} B/item)",
        load_ips * bytes_per_item as f64 / (1024.0 * 1024.0)
    );
    println!("buffer:     {load_secs:.2}s");
    println!("wall:       {wall_secs:.2}s (includes payload generation)");
    println!("peak disk:  {:.1} MiB (compressed)", mib(peak_disk));
    if peak_disk > 0 {
        println!(
            "comp ratio: {:.1}x ({:.2} B/item compressed)",
            (bytes_per_item * count) as f64 / peak_disk as f64,
            peak_disk as f64 / count as f64
        );
    }
    println!(
        "latency (µs): mean={:.1} p50={:.1} p95={:.1} p99={:.1} ({} batches)",
        load_latencies.mean(),
        load_latencies.percentile(50.0),
        load_latencies.percentile(95.0),
        load_latencies.percentile(99.0),
        load_latencies.count()
    );
    println!();

    // ------------------------------------------------------------------
    // Phase 2: RECOVER — reopen the directory (filename-based recovery).
    // ------------------------------------------------------------------
    println!("--- phase 2: recover (drop + reopen) ---");
    let t1 = Instant::now();
    let (buf, report) = SegmentBuffer::<Event>::open_with_report(&dir, config)?;
    let recover_elapsed = t1.elapsed();
    let recover_secs = recover_elapsed.as_secs_f64();
    let segs = report.segment_count;
    println!("segments:   {segs}");
    println!("disk:       {:.1} MiB", mib(report.disk_bytes));
    println!("elapsed:    {recover_secs:.3}s");
    if recover_secs > 0.0 {
        println!("seg/s:      {:.0}", segs as f64 / recover_secs);
    }
    println!();

    // ------------------------------------------------------------------
    // Phase 3: DRAIN — read_from + delete_acked (the cloud-sync loop).
    // ------------------------------------------------------------------
    println!("--- phase 3: drain (read_from + delete_acked) ---");
    let mut cursor = buf.stats().head_sequence;
    let mut seen = 0u64;
    let mut expected_id = cursor;
    let t2 = Instant::now();
    let mut drain_latencies = LatencyHistogram::new();
    let mut next_heartbeat = heartbeat;
    loop {
        let batch_t = Instant::now();
        let batch_items = buf.read_from(cursor, batch)?;
        if batch_items.is_empty() {
            break;
        }
        for item in &batch_items {
            assert_eq!(
                item.id, expected_id,
                "drain verify: id {} expected, got {} (gap or out-of-order)",
                expected_id, item.id
            );
            expected_id += 1;
        }
        let last_seq = cursor + batch_items.len() as u64 - 1;
        buf.delete_acked(last_seq)?;
        let batch_elapsed = batch_t.elapsed();
        drain_latencies.record(batch_elapsed);
        seen += batch_items.len() as u64;
        cursor = last_seq + 1;
        if seen >= next_heartbeat {
            eprintln!("  ... {seen}/{count} items drained");
            next_heartbeat += heartbeat;
        }
    }
    let drain_elapsed = t2.elapsed();
    let final_disk = buf.stats().approx_disk_bytes;

    let drain_secs = drain_elapsed.as_secs_f64();
    let drain_ips = seen as f64 / drain_secs;
    println!("items/sec:  {drain_ips:.0}");
    println!(
        "MiB/s:      {:.1} (uncompressed, est. {bytes_per_item} B/item)",
        drain_ips * bytes_per_item as f64 / (1024.0 * 1024.0)
    );
    println!("elapsed:    {drain_secs:.2}s");
    println!("final disk: {:.1} MiB (should be ~0)", mib(final_disk));
    println!(
        "latency (µs): mean={:.1} p50={:.1} p95={:.1} p99={:.1} ({} batches)",
        drain_latencies.mean(),
        drain_latencies.percentile(50.0),
        drain_latencies.percentile(95.0),
        drain_latencies.percentile(99.0),
        drain_latencies.count()
    );
    println!();

    // ------------------------------------------------------------------
    // Verify integrity.
    // ------------------------------------------------------------------
    println!("--- verify ---");
    println!("items seen: {seen}");
    println!("expected:   {count}");
    assert_eq!(
        seen, count,
        "drain verify: saw {seen} items, expected {count}"
    );
    assert_eq!(
        cursor, count,
        "drain verify: cursor {cursor}, expected {count}"
    );
    assert_eq!(final_disk, 0, "drain verify: disk not fully drained");
    println!("OK: gap-free, in-order, exactly {count} items, disk drained");

    // Clean up --dir unless --keep was passed.
    if let Some(path) = &dir_override {
        if !keep_dir {
            std::fs::remove_dir_all(path)?;
            println!("\ncleaned up: {path}");
        } else {
            println!("\nkept: {path}");
        }
    }

    Ok(())
}
