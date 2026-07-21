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
//! cargo run --release --example scaling -- [count] [batch_size] [compression]
//! cargo run --release --example scaling                       # 1M items, batch 5000, zstd-3
//! cargo run --release --example scaling -- 10000000           # 10M
//! cargo run --release --example scaling -- 100000000 10000 1  # 100M, batch 10k, zstd-1
//! ```
//!
//! # Disk estimate
//!
//! Roughly 40 compressed bytes per item (zstd-3 on sequential ids + a 64-byte
//! payload): 1M ≈ 40 MB, 10M ≈ 400 MB, 100M ≈ 4 GB. Check `df` before launching
//! 100M on a small disk. The `Throughput` durability policy (no per-flush
//! fsync) is used because this models the cloud-sync deployment where the cloud
//! is the durable layer — edit the constant below to test `Maximal`/`Segment`.

use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Fixed-size event so throughput numbers are interpretable.
///
/// Uncompressed struct size: 8 (id) + 8 (timestamp_ms) + 1 (kind) + 64
/// (payload) = 81 bytes/item. CBOR adds ~6 bytes overhead; zstd reclaims most
/// of it. The throughput report labels the MiB/s figure as "uncompressed".
#[derive(Serialize, Deserialize, Clone)]
struct Event {
    id: u64,
    timestamp_ms: u64,
    kind: u8,
    payload: String,
}

/// Uncompressed bytes per item, for the MiB/s headline.
const BYTES_PER_ITEM: u64 = 8 + 8 + 1 + 64;

/// The durability policy under test. `Throughput` (no fsync) models the
/// cloud-sync deployment. Change to `Maximal` or `Segment` to measure the
/// fsync-bound regime.
const DURABILITY: DurabilityPolicy = DurabilityPolicy::Throughput;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let batch: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000);
    let compression: i32 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let batch = batch.max(1);

    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().to_path_buf();

    println!("=== segment-buffer scaling test ===");
    println!(
        "count: {count} | batch: {batch} | compression: zstd-{compression} | durability: {DURABILITY:?}"
    );
    println!("dir: {}", dir.display());
    println!();

    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual) // one segment per explicit flush
        .max_size_bytes(u64::MAX) // no backpressure ceiling; this measures raw scaling
        .compression_level(compression)
        .durability(DURABILITY)
        .build();

    // ------------------------------------------------------------------
    // Phase 1: LOAD — append_all in batches + flush per batch.
    // ------------------------------------------------------------------
    println!("--- phase 1: load (append_all + flush per batch) ---");
    let buf = SegmentBuffer::<Event>::open(&dir, config.clone())?;
    let payload = "x".repeat(64);
    let t0 = Instant::now();
    let mut id = 0u64;
    let heartbeat = (count / 10).max(1);
    let mut next_heartbeat = heartbeat;
    while id < count {
        let take = std::cmp::min(batch as u64, count - id) as usize;
        let items: Vec<Event> = (0..take)
            .map(|i| {
                let eid = id + i as u64;
                Event {
                    id: eid,
                    timestamp_ms: eid,
                    kind: (eid % 4) as u8,
                    payload: payload.clone(),
                }
            })
            .collect();
        let last = buf.append_all(items)?;
        id = last + 1;
        buf.flush()?;
        if id >= next_heartbeat {
            eprintln!("  ... {id}/{count} items flushed");
            next_heartbeat += heartbeat;
        }
    }
    let load_elapsed = t0.elapsed();
    let peak_disk = buf.stats().approx_disk_bytes;
    assert_eq!(
        buf.latest_sequence(),
        count.saturating_sub(1),
        "load phase: latest_sequence should be count-1"
    );
    drop(buf); // release the single-process lock so we can reopen

    let load_secs = load_elapsed.as_secs_f64();
    let load_ips = count as f64 / load_secs;
    println!("items/sec:  {load_ips:.0}");
    println!(
        "MiB/s:      {:.1} (uncompressed, est. {BYTES_PER_ITEM} B/item)",
        load_ips * BYTES_PER_ITEM as f64 / (1024.0 * 1024.0)
    );
    println!("elapsed:    {load_secs:.2}s");
    println!("peak disk:  {:.1} MiB (compressed)", mib(peak_disk));
    println!();

    // ------------------------------------------------------------------
    // Phase 2: RECOVER — reopen the directory (filename-based recovery).
    // ------------------------------------------------------------------
    println!("--- phase 2: recover (drop + reopen) ---");
    let t1 = Instant::now();
    let (buf, report) = SegmentBuffer::<Event>::open_with_report(&dir, config.clone())?;
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
    let mut next_heartbeat = heartbeat;
    loop {
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
        "MiB/s:      {:.1} (uncompressed, est. {BYTES_PER_ITEM} B/item)",
        drain_ips * BYTES_PER_ITEM as f64 / (1024.0 * 1024.0)
    );
    println!("elapsed:    {drain_secs:.2}s");
    println!("final disk: {:.1} MiB (should be ~0)", mib(final_disk));
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

    Ok(())
}
