use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct TestItem {
    id: u64,
    payload: String,
}

fn test_item(n: u64) -> TestItem {
    TestItem {
        id: n,
        payload: format!("payload-{n}"),
    }
}

type TestBuffer = SegmentBuffer<TestItem>;

fn test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            max_batch_events: 4,
            flush_interval_secs: 3600,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            cipher: None,
        },
    )
    .expect("Failed to create buffer")
}

/// Buffer with max_size_bytes=1000 so pressure percentages are exact.
fn pressure_test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            max_batch_events: 4,
            flush_interval_secs: 3600,
            max_size_bytes: 1000,
            compression_level: 3,
            cipher: None,
        },
    )
    .expect("Failed to create pressure-test buffer")
}

fn set_disk_bytes<T>(buf: &SegmentBuffer<T>, bytes: u64) {
    let mut inner = buf.inner.lock();
    inner.approx_disk_bytes = bytes;
}

// =========================================================================
// Filename parsing
// =========================================================================

#[test]
fn parse_filename_roundtrip() {
    let range = parse_segment_filename("seg_000000000000_000000000255.zst").unwrap();
    assert_eq!(range.start, 0);
    assert_eq!(range.end, 255);

    let range = parse_segment_filename("seg_000000001000_000000001099.zst").unwrap();
    assert_eq!(range.start, 1000);
    assert_eq!(range.end, 1099);

    assert!(parse_segment_filename("not_a_segment").is_none());
    assert!(parse_segment_filename("seg_000000000000.zst").is_none());
}

// =========================================================================
// Basic append / flush / read
// =========================================================================

#[test]
fn append_and_flush() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }
    assert_eq!(buf.pending_count(), 3);

    buf.flush().unwrap();
    assert_eq!(buf.pending_count(), 3);

    let segments = buf.scan_segments().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].start, 0);
    assert_eq!(segments[0].end, 2);
}

#[test]
fn auto_flush_at_batch_threshold() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let segments = buf.scan_segments().unwrap();
    assert_eq!(segments.len(), 1, "Should auto-flush at batch threshold");
}

#[test]
fn read_from_returns_flushed_events() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 5);
}

#[test]
fn read_from_partial_segment() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(2, 100).unwrap();
    assert_eq!(events.len(), 2, "Should skip first 2 events in segment");
}

#[test]
fn read_from_with_limit() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..6 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 3).unwrap();
    assert_eq!(events.len(), 3);
}

#[test]
fn delete_acked_removes_segments() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..8 {
        buf.append(test_item(i)).unwrap();
    }

    let deleted = buf.delete_acked(3).unwrap();
    assert_eq!(deleted, 1, "Should delete segment [0-3]");

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 4, "Should only have events 4-7");
}

#[test]
fn delete_acked_all() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let deleted = buf.delete_acked(3).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(buf.pending_count(), 0);
}

#[test]
fn latest_sequence() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    assert_eq!(buf.latest_sequence(), 0);

    buf.append(test_item(0)).unwrap();
    assert_eq!(buf.latest_sequence(), 0);

    buf.append(test_item(1)).unwrap();
    assert_eq!(buf.latest_sequence(), 1);
}

// =========================================================================
// Crash recovery
// =========================================================================

#[test]
fn crash_recovery_from_segments() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = test_buffer(dir);
        for i in 0..6 {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }

    let buf2 = test_buffer(dir);
    assert_eq!(buf2.pending_count(), 6);
    assert_eq!(buf2.latest_sequence(), 5);

    let events = buf2.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 6);
}

#[test]
fn crash_recovery_loses_unflushed_events() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = test_buffer(dir);
        for i in 0..6 {
            buf.append(test_item(i)).unwrap();
        }
    }

    let buf2 = test_buffer(dir);
    assert_eq!(
        buf2.pending_count(),
        4,
        "Should only recover flushed events (pending batch lost on crash)"
    );
    assert_eq!(buf2.latest_sequence(), 3);
}

#[test]
fn crash_recovery_cleans_tmp_files() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    fs::write(
        dir.join("seg_000000000000_000000000003.zst.tmp"),
        b"incomplete",
    )
    .unwrap();

    let buf = test_buffer(dir);
    assert_eq!(buf.pending_count(), 0);
    assert!(!dir.join("seg_000000000000_000000000003.zst.tmp").exists());
}

// =========================================================================
// Roundtrip integrity
// =========================================================================

#[test]
fn read_includes_pending_events() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    for i in 4..7 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 7, "Should include 4 flushed + 3 pending");
}

#[test]
fn roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    let item = test_item(42);
    buf.append(item.clone()).unwrap();
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], item);
}

// =========================================================================
// Pressure / overload (store_pressure stays in the crate; should_accept is removed)
// =========================================================================

#[test]
fn store_pressure_returns_0_when_no_limit() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            max_batch_events: 4,
            flush_interval_secs: 3600,
            max_size_bytes: 0,
            compression_level: 3,
            cipher: None,
        },
    )
    .expect("create buffer");
    assert_eq!(buf.store_pressure(), 0.0);
    assert!(!buf.is_overloaded());
}

#[test]
fn store_pressure_bounded_at_1_0_when_disk_exceeds_limit() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            max_batch_events: 4,
            flush_interval_secs: 3600,
            max_size_bytes: 1,
            compression_level: 3,
            cipher: None,
        },
    )
    .expect("create buffer");
    set_disk_bytes(&buf, 999_999_999);
    let pressure = buf.store_pressure();
    assert!(
        (pressure - 1.0).abs() < f32::EPSILON,
        "Pressure should be clamped to 1.0, got {pressure}"
    );
    assert!(buf.is_overloaded());
}

#[test]
fn is_overloaded_true_above_90_percent() {
    let tmp = TempDir::new().unwrap();
    let buf = pressure_test_buffer(tmp.path());
    set_disk_bytes(&buf, 901); // 90.1%
    assert!(buf.is_overloaded());
}

#[test]
fn is_overloaded_false_at_or_below_90_percent() {
    let tmp = TempDir::new().unwrap();
    let buf = pressure_test_buffer(tmp.path());
    set_disk_bytes(&buf, 900); // exactly 90%
    assert!(
        !buf.is_overloaded(),
        "is_overloaded is pressure > 0.9, not >="
    );
}

// =========================================================================
// Concurrency stress test — 4 writers + 1 reader, 10K events
// =========================================================================

#[test]
fn concurrency_4_writers_1_reader_10k_events() {
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 2_500;
    const TOTAL: usize = WRITERS * PER_WRITER; // 10_000

    let latest_seen = Arc::new(Mutex::new(0u64));

    thread::scope(|s| {
        // Reader thread: polls read_from until all events seen
        let buf_r = Arc::clone(&buf);
        let latest_r = Arc::clone(&latest_seen);
        s.spawn(move || loop {
            let start = *latest_r.lock();
            if start >= TOTAL as u64 {
                break;
            }
            if let Ok(events) = buf_r.read_from(start, 500) {
                if !events.is_empty() {
                    *latest_r.lock() = start + events.len() as u64;
                }
            }
            thread::sleep(Duration::from_micros(50));
        });

        // 4 writer threads, each appending 2_500 events
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            s.spawn(move || {
                for i in 0..PER_WRITER {
                    let _ = buf_w.append(test_item((writer_id * PER_WRITER + i) as u64));
                }
            });
        }
    });

    // All threads joined. Flush any remaining in-memory events.
    buf.flush().unwrap();

    // Verify: all 10K events assigned, all recoverable
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);
    assert_eq!(buf.pending_count(), TOTAL as u64);

    let all_events = buf.read_from(0, TOTAL * 2).unwrap();
    assert_eq!(
        all_events.len(),
        TOTAL,
        "All {TOTAL} events should be recoverable"
    );
}

// =========================================================================
// Time-based auto-flush
// =========================================================================

#[test]
fn time_based_auto_flush() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            max_batch_events: 256,
            flush_interval_secs: 1,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            cipher: None,
        },
    )
    .expect("create buffer");

    buf.append(test_item(0)).unwrap();
    assert!(
        buf.scan_segments().unwrap().is_empty(),
        "Event should remain in memory, not flushed yet"
    );

    thread::sleep(Duration::from_millis(1100));
    buf.append(test_item(1)).unwrap();

    let segments = buf.scan_segments().unwrap();
    assert!(
        !segments.is_empty(),
        "Time-based flush should have created a segment file"
    );
}

// =========================================================================
// Error-path tests (no encryption)
// =========================================================================

#[test]
fn corrupted_zstd_segment_returns_error_not_panic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let garbage_path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&garbage_path, b"this is not valid zstd data at all").unwrap();

    let buf = test_buffer(dir);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Corrupted zstd segment should return an error, not panic"
    );
}

// =========================================================================
// Encryption tests (behind `encryption` feature)
// =========================================================================

#[cfg(feature = "encryption")]
fn encrypted_buffer(dir: &Path, key: [u8; 32]) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            max_batch_events: 4,
            flush_interval_secs: 3600,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            cipher: Some(Box::new(AesGcmCipher::new(&key))),
        },
    )
    .expect("Failed to create encrypted buffer")
}

#[cfg(feature = "encryption")]
#[test]
fn encrypted_roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer(tmp.path(), [0u8; 32]);

    let item = test_item(99);
    buf.append(item.clone()).unwrap();
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], item);

    // Verify the segment file on disk is NOT plaintext
    let raw = fs::read(
        tmp.path()
            .read_dir()
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap();
    assert!(
        raw.len() > 12,
        "Encrypted segment should be nonce + ciphertext, not plaintext"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn truncated_encrypted_segment_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&path, [0u8; 11]).unwrap();

    let buf = encrypted_buffer(dir, [0u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Truncated encrypted segment (<12 bytes) should return an error"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn encrypted_segment_nonce_only_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&path, [0u8; 12]).unwrap();

    let buf = encrypted_buffer(dir, [0u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Encrypted segment with nonce but no ciphertext should return an error"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn wrong_decryption_key_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    let buf = encrypted_buffer(dir, [1u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Wrong decryption key should fail to read encrypted segment"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn decrypt_without_key_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    // Reopen WITHOUT cipher — tries to zstd-decode ciphertext → fails
    let buf = test_buffer(dir);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Reading encrypted segment without a cipher should fail"
    );
}
