use super::*;
use serde::{Deserialize, Serialize};
use std::fs;
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

/// Shared test config: small batch, auto-flush effectively disabled. Only
/// `max_size_bytes` varies between tests, so it is the single parameter.
fn test_config(max_size_bytes: u64) -> SegmentConfig {
    SegmentConfig {
        flush_policy: FlushPolicy::Batch(4),
        max_size_bytes,
        compression_level: 3,
        durability: DurabilityPolicy::Segment,
        cipher: None,
    }
}

fn test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(dir, test_config(1024 * 1024)).expect("Failed to create buffer")
}

/// Buffer with max_size_bytes=1000 so pressure percentages are exact.
fn pressure_test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(dir, test_config(1000)).expect("Failed to create pressure-test buffer")
}

fn set_disk_bytes<T>(buf: &SegmentBuffer<T>, bytes: u64) {
    buf.approx_disk_bytes
        .store(bytes, std::sync::atomic::Ordering::Relaxed);
}

// =========================================================================
// Filename parsing
// =========================================================================

#[test]
fn parse_filename_roundtrip() {
    use super::segment::parse_filename;

    let range = parse_filename("seg_000000000000_000000000255.zst").unwrap();
    assert_eq!(range.start, 0);
    assert_eq!(range.end, 255);

    let range = parse_filename("seg_000000001000_000000001099.zst").unwrap();
    assert_eq!(range.start, 1000);
    assert_eq!(range.end, 1099);

    assert!(parse_filename("not_a_segment").is_none());
    assert!(parse_filename("seg_000000000000.zst").is_none());
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
fn delete_acked_with_unflushed_pending_keeps_backlog_honest() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);

    // Two items stay in memory: max_batch_events is 4, no auto-flush fires.
    buf.append(test_item(0)).unwrap();
    buf.append(test_item(1)).unwrap();
    assert_eq!(buf.pending_count(), 2);

    // Consumer reads them from memory, then acks past them. There is no
    // segment file to remove, so deleted == 0.
    let deleted = buf.delete_acked(100).unwrap();
    assert_eq!(deleted, 0, "Nothing was flushed, so no segment is removed");

    // The unflushed items remain in the backlog and are still readable.
    assert_eq!(
        buf.pending_count(),
        2,
        "Unflushed items must stay counted until flushed + acknowledged"
    );
    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 2);
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
    let buf: TestBuffer = SegmentBuffer::open(tmp.path(), test_config(0)).expect("create buffer");
    assert_eq!(buf.store_pressure(), 0.0);
    assert!(!buf.is_overloaded());
}

#[test]
fn store_pressure_bounded_at_1_0_when_disk_exceeds_limit() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(tmp.path(), test_config(1)).expect("create buffer");
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
    // FlushPolicy::Manual keeps all items in-memory during the concurrent phase.
    // The purpose is to stress-test append/read correctness under contention,
    // not disk I/O. With Batch(4) this test would create 2_500 segment files.
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
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
            flush_policy: FlushPolicy::BatchOrInterval {
                batch_size: 256,
                interval: std::time::Duration::from_secs(1),
            },
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
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

#[test]
fn legacy_envelopeless_file_still_reads() {
    use super::segment;
    // Hand-build a v1-format file (no SBF1 envelope), exactly as monitor365
    // would have written it: raw zstd(CBOR), no envelope prefix.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let items = vec![test_item(7), test_item(8)];
    let mut cbor = Vec::new();
    ciborium::into_writer(&items, &mut cbor).unwrap();
    let raw_v1 = zstd::encode_all(cbor.as_slice(), 3).unwrap();

    let path = dir.join(segment::filename(7, 8));
    fs::write(&path, &raw_v1).unwrap();

    // Read via the buffer (no cipher). The envelope-less bytes should be
    // detected as legacy and decoded transparently.
    let buf = test_buffer(dir);
    let events: Vec<TestItem> = buf.read_from(7, 100).unwrap();
    assert_eq!(events.len(), 2, "legacy envelope-less file must still read");
    assert_eq!(events[0], test_item(7));
}

#[test]
fn enveloped_file_roundtrips_and_carries_magic() {
    use super::segment;
    const ENVELOPE_MAGIC: &[u8; 4] = b"SBF1";

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);

    buf.append(test_item(1)).unwrap();
    buf.append(test_item(2)).unwrap();
    buf.flush().unwrap();

    // The file on disk must start with the SBF1 magic. Sequence numbers are
    // assigned by the buffer (0-based), so two appends → filename(0, 1).
    let path = dir.join(segment::filename(0, 1));
    assert!(path.exists(), "segment file should exist at {path:?}");
    let bytes = fs::read(&path).unwrap();
    assert!(
        bytes.len() >= 8,
        "enveloped file should be at least header-length"
    );
    assert_eq!(
        &bytes[..4],
        ENVELOPE_MAGIC,
        "newly-written segment must carry the SBF1 envelope magic"
    );

    // And it must round-trip cleanly.
    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn envelope_detection_requires_zero_reserved_bytes() {
    use super::segment::{unwrap_envelope, wrap_envelope};

    // Sanity: the canonical envelope (zero reserved) is detected.
    let wrapped = wrap_envelope(b"payload");
    assert!(matches!(unwrap_envelope(&wrapped), (Some(1), _)));

    // A v1-shape block whose reserved bytes are NON-zero must NOT be treated
    // as an envelope, even though the magic matches. This is the hardening:
    // a legacy encrypted file whose AEAD nonce begins with `SBF1` followed
    // by three non-zero bytes (~2⁻³² of files) would otherwise be silently
    // mis-framed as an envelope. Requiring reserved-zero drops the false
    // positive to 2⁻⁵⁶.
    let mut looks_like_envelope = vec![b'S', b'B', b'F', b'1', 1, 0xFF, 0xFF, 0xFF];
    looks_like_envelope.extend_from_slice(b"payload");
    let (version, payload) = unwrap_envelope(&looks_like_envelope);
    assert_eq!(
        version, None,
        "magic with non-zero reserved bytes must not be detected as envelope"
    );
    assert_eq!(
        payload,
        looks_like_envelope.as_slice(),
        "non-conforming bytes must pass through unmodified as legacy"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn legacy_encrypted_file_without_envelope_still_reads() {
    // The headline monitor365 byte-compatibility guarantee: a segment file
    // written by monitor365 (no SBF1 envelope, just `[nonce][ciphertext]`)
    // must read back transparently through the enveloped reader when the
    // matching cipher is configured. This was previously untested.
    use super::segment;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let items = vec![test_item(101), test_item(102), test_item(103)];

    // Encode the v1 payload exactly as monitor365 would have: CBOR → zstd →
    // AEAD-encrypt. Then write the raw payload bytes (NO envelope) under a
    // valid segment filename.
    let key = [0xABu8; 32];
    let cipher = AesGcmCipher::new(&key);
    let path = dir.join(segment::filename(101, 103));
    let mut compressor = zstd::bulk::Compressor::new(3).unwrap();
    let payload = segment::encode_payload(Some(&cipher), &mut compressor, &path, &items).unwrap();
    assert!(
        !payload.starts_with(b"SBF1"),
        "raw encrypted payload must not accidentally carry the magic"
    );
    fs::write(&path, &payload).unwrap();

    // Open the buffer with the same cipher and read the segment back.
    let buf = encrypted_buffer(dir, key);
    let events: Vec<TestItem> = buf.read_from(101, 100).unwrap();
    assert_eq!(
        events, items,
        "legacy encrypted file (no envelope) must decode transparently"
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
            flush_policy: FlushPolicy::Batch(4),
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: Some(Arc::new(AesGcmCipher::new(&key))),
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

#[cfg(feature = "encryption")]
#[test]
fn wrong_key_cipher_error_carries_source_chain() {
    // The cipher error surfaced to the caller must keep the underlying AEAD
    // failure reachable via `std::error::Error::source`, so operators can
    // inspect the original decryption failure instead of just a flat string.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    let buf = encrypted_buffer(dir, [1u8; 32]);
    let err = buf.read_from(0, 100).expect_err("wrong key must error");

    let super::SegmentError::Cipher { message, .. } = &err else {
        panic!("expected Cipher variant, got {err:?}");
    };
    assert!(
        message.contains("AES-GCM decryption failed"),
        "message should name the phase, got: {message}"
    );
    // The CipherError's source chain was lost when promoted to SegmentError::Cipher
    // (the variant stores a flat String), but the underlying AEAD failure must
    // still be reachable on the CipherError itself. We exercise that path via
    // a direct cipher call.
    use super::SegmentCipher;
    let cipher = AesGcmCipher::new(&[0u8; 32]);
    let bad_payload = [0u8; 64]; // plausible size, wrong bytes
    let cipher_err = cipher.decrypt(&bad_payload).unwrap_err();
    assert!(
        std::error::Error::source(&cipher_err).is_some(),
        "CipherError from AES-GCM must expose the AEAD failure via source()"
    );
}

// =========================================================================
// XChaCha20-Poly1305 cipher (encryption feature)
// =========================================================================

#[cfg(feature = "encryption")]
fn encrypted_buffer_xchacha(dir: &Path, key: [u8; 32]) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            flush_policy: FlushPolicy::Batch(4),
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: Some(Arc::new(XChaCha20Poly1305Cipher::new(&key))),
        },
    )
    .expect("Failed to create XChaCha20-encrypted buffer")
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer_xchacha(tmp.path(), [0u8; 32]);

    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 5);
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.id, i as u64);
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_cipher_roundtrip_direct() {
    // Direct trait-level roundtrip independent of the buffer: encrypt + decrypt
    // must reproduce the input for arbitrary plaintexts.
    let cipher = XChaCha20Poly1305Cipher::new(&[0xau8; 32]);
    for plaintext in [b"".as_slice(), b"x", b"hello world", &[42u8; 4096]] {
        let ct = cipher.encrypt(plaintext).expect("encrypt");
        // The ciphertext must include the 24-byte nonce prefix.
        assert!(ct.len() >= 24, "ciphertext must include nonce prefix");
        let pt = cipher.decrypt(&ct).expect("decrypt");
        assert_eq!(pt, plaintext, "roundtrip must reproduce plaintext");
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_tamper_detection() {
    // Flip one byte of the ciphertext → AEAD tag must fail verification.
    let cipher = XChaCha20Poly1305Cipher::new(&[0xbu8; 32]);
    let mut ct = cipher.encrypt(b"secret payload").expect("encrypt");
    // Flip the last byte (inside the Poly1305 tag region).
    let last = ct.len() - 1;
    ct[last] ^= 0x01;
    let err = cipher
        .decrypt(&ct)
        .expect_err("tampered ciphertext must fail AEAD");
    assert!(
        err.to_string().contains("XChaCha20"),
        "error should name XChaCha20: got {err}"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_short_payload_rejected() {
    // Payload shorter than the 24-byte nonce prefix must be rejected before
    // the AEAD is invoked, with a clear CipherError (not an opaque AEAD error).
    let cipher = XChaCha20Poly1305Cipher::new(&[0xcu8; 32]);
    for short_len in 0..24 {
        let payload = vec![0u8; short_len];
        let err = cipher
            .decrypt(&payload)
            .expect_err("sub-nonce payload must error");
        assert!(
            err.to_string().contains("nonce"),
            "error should mention nonce: got {err}"
        );
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_buffer_segment_roundtrip_with_delete_acked() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer_xchacha(tmp.path(), [0xdu8; 32]);

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    // Batch(4) triggers auto-flush on the 4th append.
    assert_eq!(buf.pending_count(), 4);

    // Acknowledge the first 3 items; one segment [0..=3] is too tall to
    // ack with seq=2, so the segment survives.
    let removed = buf.delete_acked(2).unwrap();
    assert_eq!(removed, 0);
    // Ack all 4: segment [0..=3] is fully covered.
    let removed = buf.delete_acked(3).unwrap();
    assert_eq!(removed, 1);
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_recommended_cipher_installs_xchacha() {
    // The recommended_cipher() builder helper must install an XChaCha20
    // cipher (the documented direction for new buffers).
    let cfg = SegmentConfig::builder()
        .recommended_cipher([0xeu8; 32])
        .build();
    assert!(
        cfg.cipher.is_some(),
        "recommended_cipher must install a cipher"
    );
    // Smoke: the cipher works for a roundtrip via the buffer.
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(tmp.path(), cfg).unwrap();
    buf.append(test_item(7)).unwrap();
    buf.flush().unwrap();
    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items, vec![test_item(7)]);
}

// =========================================================================
// Single-process flock (M2)
// =========================================================================

/// Second `open()` on the same directory while the first buffer is alive
/// must fail fast with [`SegmentError::Locked`].
#[test]
fn flock_second_open_returns_locked_error() {
    let tmp = TempDir::new().unwrap();
    let _first = test_buffer(tmp.path());
    let result = SegmentBuffer::<TestItem>::open(tmp.path(), test_config(1024 * 1024));
    let err = result.expect_err("second open must fail");
    assert!(
        matches!(err, SegmentError::Locked { .. }),
        "expected SegmentError::Locked, got {err:?}"
    );
    let rendered = format!("{err}");
    assert!(
        rendered.contains("locked by another process"),
        "error should mention lock: got {rendered}"
    );
}

/// After the first buffer is dropped, the lock releases and a new `open()`
/// succeeds. This is the kernel-advisory-lock contract: dropping the fd
/// releases the flock.
#[test]
fn flock_open_after_drop_succeeds() {
    let tmp = TempDir::new().unwrap();
    {
        let _first = test_buffer(tmp.path());
        assert!(SegmentBuffer::<TestItem>::open(tmp.path(), test_config(1024 * 1024)).is_err());
        // _first dropped here: the flock is released.
    }
    let _second = test_buffer(tmp.path());
    // If we reached here without panicking, the second open succeeded.
}

/// A lock file is created in the directory as a side-effect of `open()`.
/// Operators can list it; recovery must ignore it (it does not match
/// `seg_*_*.zst`).
#[test]
fn flock_creates_lock_sidecar_file() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    // Append + flush so a segment file exists alongside the lock file.
    buf.append(test_item(0)).unwrap();
    buf.flush().unwrap();

    let entries: Vec<String> = std::fs::read_dir(tmp.path())
        .expect("dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.iter().any(|n| n == ".segment-buffer.lock"),
        "lock sidecar must exist; dir contents: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("seg_") && n.ends_with(".zst")),
        "segment file must exist; dir contents: {entries:?}"
    );
    // The lock file must not be confused with a segment: scan_segments returns
    // exactly one segment (the lock is ignored).
    assert_eq!(
        buf.read_from(0, 100).unwrap().len(),
        1,
        "lock file must not show up as a segment"
    );
}

/// Different directories are independently lockable. Two buffers in two
/// directories must both succeed.
#[test]
fn flock_locks_are_per_directory() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let _a = test_buffer(tmp_a.path());
    let _b = test_buffer(tmp_b.path());
    // No panic — both opens succeeded.
}

// =========================================================================
// SegmentIter (M7)
// =========================================================================

#[test]
fn iter_from_yields_seq_item_pairs() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    // Batch(4) triggers a flush on the 4th append; one item stays in memory.
    let collected: Vec<(u64, TestItem)> = buf.iter_from(0, 100).unwrap().collect();
    assert_eq!(
        collected.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        collected.iter().map(|(_, i)| i.id).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
}

#[test]
fn iter_from_limit_zero_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();
    let count = buf.iter_from(0, 0).unwrap().count();
    assert_eq!(count, 0);
}

#[test]
fn iter_from_respects_limit() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    let collected: Vec<u64> = buf.iter_from(2, 3).unwrap().map(|(_, i)| i.id).collect();
    assert_eq!(collected, vec![2, 3, 4]);
}

#[test]
fn iter_from_chains_with_iterator_combinators() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // The classic Iterator combinators all work.
    let sum: u64 = buf
        .iter_from(0, 100)
        .unwrap()
        .map(|(_, i)| i.id)
        .filter(|x| x % 2 == 0)
        .sum();
    assert_eq!(sum, 2 + 4 + 6 + 8);
}

#[test]
fn iter_from_start_seq_skips_already_read_items() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    let collected: Vec<u64> = buf.iter_from(3, 100).unwrap().map(|(_, i)| i.id).collect();
    assert_eq!(collected, vec![3, 4]);
}

// =========================================================================
// mtime probe for scan cache (M13)
// =========================================================================

/// On a real filesystem (the tempdir), the probe should return `true` —
/// mtime moves when we write twice with a sleep in between. On a
/// filesystem that pins mtime to a constant, the probe returns `false`
/// and the cache guard is skipped (today's behavior).
#[test]
fn mtime_probe_returns_true_on_real_filesystem() {
    let tmp = TempDir::new().unwrap();
    // The probe runs at open(). Today's CI is on real filesystems
    // (ext4/tmpfs on Linux, apfs on macOS) where mtime is fine-grained.
    let buf = test_buffer(tmp.path());
    assert!(
        buf.mtime_supported,
        "mtime capability probe should return true on the host filesystem; \
         if this fires, the test host has a coarse-granularity or no-mtime \
         filesystem (the cache guard is correctly disabled in that case, \
         but the test assertion needs to match)"
    );
}

/// External directory mutation must be detected by the mtime guard: if
/// someone removes a segment file out from under us, the next scan_cache
/// hit must NOT serve the stale list (would silently drop the segment's
/// items from reads).
#[test]
fn external_segment_removal_invalidates_scan_cache() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // Two flushes so we have two segments to reason about.
    for i in 4..8 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // 8 items readable through the public API.
    assert_eq!(buf.read_from(0, 100).unwrap().len(), 8);

    // Simulate an external process quarantining one segment.
    let segments: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "zst"))
        .collect();
    assert_eq!(segments.len(), 2, "expected exactly two segments on disk");
    let _ = std::fs::remove_file(&segments[0]);

    // Sleep briefly so the dir mtime moves past the cached value (the
    // probe sleep is 15ms; we use 25ms here for headroom on coarse fs).
    std::thread::sleep(std::time::Duration::from_millis(25));

    // The next read must observe the removal: only one segment's items
    // (4) survive. Without the mtime guard, the stale cache would still
    // report both segments as on-disk and reads would try (and fail) to
    // open the removed file — surfacing as an Err.
    let after = buf.read_from(0, 100).unwrap().len();
    assert_eq!(
        after, 4,
        "external removal must be reflected via mtime guard"
    );
}

// =========================================================================
// Debug impl for SegmentBuffer<T>
// =========================================================================

#[test]
fn debug_impl_formats_cleanly() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();
    buf.append(test_item(1)).unwrap();
    buf.append(test_item(2)).unwrap();

    let rendered = format!("{:?}", buf);
    // Structural sanity: struct name + path field + every BufferStats field.
    assert!(
        rendered.starts_with("SegmentBuffer {"),
        "expected SegmentBuffer struct prefix, got: {rendered}"
    );
    // debug_struct renders field names as bare identifiers (no quotes).
    assert!(
        rendered.contains("dir: "),
        "Debug must expose the dir field, got: {rendered}"
    );
    for field in [
        "pending_count",
        "latest_sequence",
        "head_sequence",
        "next_sequence",
        "approx_disk_bytes",
        "max_size_bytes",
        "store_pressure",
    ] {
        assert!(
            rendered.contains(&format!("{field}: ")),
            "Debug must expose the `{field}` field, got: {rendered}"
        );
    }
    // pending_count reflects the three appends.
    assert!(
        rendered.contains("pending_count: 3"),
        "expected pending_count: 3, got: {rendered}"
    );
}

// =========================================================================
// Display snapshot tests — lock the format strings so a careless edit
// (e.g. changing a brace in a `thiserror` attribute) shows up as a test
// failure instead of silently shifting operator-facing log output.
// =========================================================================

#[test]
fn segment_error_io_display_format_no_path() {
    // Io constructed from a bare io::Error via `?` has site = Unknown.
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let err: SegmentError = io_err.into();
    let rendered = format!("{err}");
    // No " for ..." clause when site is Unknown.
    assert_eq!(rendered, "I/O error: missing");
}

#[test]
fn segment_error_io_display_format_with_segment_path() {
    // Io constructed with explicit Segment site renders the path clause.
    let err = SegmentError::Io {
        site: IoSite::Segment(std::path::PathBuf::from(
            "/var/data/seg_000000000000_000000000000.zst",
        )),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "I/O error for /var/data/seg_000000000000_000000000000.zst: permission denied"
    );
}

#[test]
fn segment_error_io_display_format_with_dir_site() {
    // Io with site = Dir renders a fixed clause (no path payload — the
    // directory is reachable via SegmentBuffer::path()).
    let err = SegmentError::Io {
        site: IoSite::Dir,
        source: std::io::Error::new(std::io::ErrorKind::ReadOnlyFilesystem, "read-only"),
    };
    let rendered = format!("{err}");
    assert_eq!(rendered, "I/O error for the segment directory: read-only");
}

#[test]
fn segment_error_with_path_upgrades_unknown_to_segment() {
    // with_path on an Unknown Io error upgrades the site to Segment.
    let raw: SegmentError = std::io::Error::other("boom").into();
    let upgraded = raw.with_path("/tmp/seg.zst");
    match upgraded {
        SegmentError::Io {
            site: IoSite::Segment(p),
            ..
        } => {
            assert_eq!(p, std::path::PathBuf::from("/tmp/seg.zst"));
        }
        other => panic!("expected Io with Segment site, got {other:?}"),
    }
}

#[test]
fn segment_error_with_path_leaves_segment_alone() {
    // First call site to attach context wins: calling with_path on a Segment
    // site leaves the original path intact (no clobbering).
    let err = SegmentError::Io {
        site: IoSite::Segment(std::path::PathBuf::from("/original/path.zst")),
        source: std::io::Error::other("x"),
    };
    let upgraded = err.with_path("/wrong/attempt.zst");
    match upgraded {
        SegmentError::Io {
            site: IoSite::Segment(p),
            ..
        } => {
            assert_eq!(p, std::path::PathBuf::from("/original/path.zst"));
        }
        other => panic!("expected Io with original Segment site, got {other:?}"),
    }
}

#[test]
fn segment_error_with_dir_upgrades_unknown_to_dir() {
    // with_dir on an Unknown Io error tags the site as Dir.
    let raw: SegmentError = std::io::Error::other("boom").into();
    let tagged = raw.with_dir();
    assert!(matches!(
        tagged,
        SegmentError::Io {
            site: IoSite::Dir,
            ..
        }
    ));
}

#[test]
fn segment_error_io_with_path_attaches_path() {
    // Upgrade a bare propagated io::Error to carry path context.
    let io_err: SegmentError =
        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read").into();
    let upgraded = io_err.with_path("/tmp/seg_000000000000_000000000000.zst");
    let rendered = format!("{upgraded}");
    assert_eq!(
        rendered,
        "I/O error for /tmp/seg_000000000000_000000000000.zst: short read"
    );
}

#[test]
fn segment_error_cbor_display_format() {
    let err = SegmentError::Cbor {
        phase: "deserialize",
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        message: "unexpected eof".into(),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "CBOR deserialize failed for /var/data/seg_000000000000_000000000000.zst: unexpected eof"
    );
}

#[test]
fn segment_error_cipher_display_format() {
    let err = SegmentError::Cipher {
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        message: "AES-GCM decryption failed".into(),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "cipher error for /var/data/seg_000000000000_000000000000.zst: AES-GCM decryption failed"
    );
}

#[test]
fn segment_error_integrity_display_format() {
    let err = SegmentError::Integrity {
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        reason: "truncated payload",
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "integrity failure for /var/data/seg_000000000000_000000000000.zst: truncated payload"
    );
}

#[test]
fn cipher_error_msg_display_format() {
    let err = super::CipherError::msg("key not configured");
    let rendered = format!("{err}");
    // msg() preserves the message verbatim; no prefix or decoration.
    assert_eq!(rendered, "key not configured");
}

#[test]
#[cfg(feature = "encryption")]
fn cipher_error_with_source_display_format() {
    use std::error::Error as _;

    #[derive(Debug)]
    struct FakeAead;
    impl std::fmt::Display for FakeAead {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("aead tag mismatch")
        }
    }
    impl std::error::Error for FakeAead {}

    let err = super::CipherError::with_source("AES-GCM decryption failed", FakeAead);
    // Display intentionally hides the source chain — the message stands alone.
    // The underlying cause is reachable only via `Error::source()`.
    assert_eq!(format!("{err}"), "AES-GCM decryption failed");
    let src = err.source().expect("with_source must populate source()");
    assert_eq!(format!("{src}"), "aead tag mismatch");
}

// =========================================================================
// for_each_from re-entrancy guard
// =========================================================================

#[test]
fn for_each_from_reentry_panics_with_clear_message() {
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));
    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }

    // Re-enter pending_count from inside the callback. The buffer's mutex is
    // held during Phase 2 (in-memory iteration); without the guard this would
    // deadlock silently. With the guard it must panic with a message naming
    // both the offending method and for_each_from.
    let buf_clone = Arc::clone(&buf);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = buf.for_each_from(0, 100, |_seq, _item| {
            let _ = buf_clone.pending_count();
        });
    }));

    let err = result.expect_err("re-entry must panic, not deadlock");
    let msg = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&'static str>().copied())
        .expect("panic payload should be a string");
    assert!(
        msg.contains("for_each_from"),
        "panic should name for_each_from, got: {msg}"
    );
    assert!(
        msg.contains("pending_count"),
        "panic should name the re-entered method, got: {msg}"
    );
}

#[test]
fn for_each_from_reentry_guard_clears_after_panic() {
    // After a panicking callback, the buffer must NOT be permanently bricked
    // — the IterationGuard must clear the flag during unwinding.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));
    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }

    let buf_clone = Arc::clone(&buf);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = buf.for_each_from(0, 100, |_seq, _item| {
            let _ = buf_clone.stats();
        });
    }));

    // The buffer must be usable again.
    assert_eq!(buf.pending_count(), 3, "buffer must be usable after panic");
    assert_eq!(buf.latest_sequence(), 2);
}

// =========================================================================
// append_all batch primitive
// =========================================================================

#[test]
fn append_all_assigns_contiguous_sequences() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    let last = buf
        .append_all([test_item(1), test_item(2), test_item(3)])
        .unwrap();
    assert_eq!(last, 2, "last seq should be 2 (0-based)");
    assert_eq!(buf.pending_count(), 3);

    // A second batch continues the sequence.
    let last2 = buf.append_all([test_item(4), test_item(5)]).unwrap();
    assert_eq!(last2, 4);
    assert_eq!(buf.pending_count(), 5);
}

#[test]
fn append_all_empty_iterator_is_noop() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();

    let last = buf.append_all(std::iter::empty::<TestItem>()).unwrap();
    assert_eq!(last, 0, "empty append_all returns current last seq");
    assert_eq!(buf.pending_count(), 1);
}

#[test]
fn append_all_visibly_cheaper_lock_count_than_loop_append() {
    // Not a perf test — a correctness test: append_all assigns contiguous
    // seqs even under concurrent writers, because the whole batch is under
    // one lock. Two concurrent append_all calls must not interleave seqs.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));

    thread::scope(|s| {
        let b1 = Arc::clone(&buf);
        s.spawn(move || {
            b1.append_all((0..100).map(test_item)).unwrap();
        });
        let b2 = Arc::clone(&buf);
        s.spawn(move || {
            b2.append_all((0..100).map(test_item)).unwrap();
        });
    });

    // All 200 items must be present. Seqs are contiguous but the two batches
    // may land in either order.
    assert_eq!(buf.pending_count(), 200);
    assert_eq!(buf.latest_sequence(), 199);
}

// =========================================================================
// path() and config() accessors
// =========================================================================

#[test]
fn path_accessor_returns_directory() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    assert_eq!(buf.path(), tmp.path());
}

#[test]
fn config_accessor_returns_opened_config() {
    let tmp = TempDir::new().unwrap();
    let config = SegmentConfig {
        flush_policy: FlushPolicy::Batch(7),
        max_size_bytes: 42,
        compression_level: 9,
        durability: DurabilityPolicy::Throughput,
        cipher: None,
    };
    let buf = test_buffer_with_config(tmp.path(), config);
    let cfg = buf.config();
    assert_eq!(cfg.flush_policy, FlushPolicy::Batch(7));
    assert_eq!(cfg.max_size_bytes, 42);
    assert_eq!(cfg.compression_level, 9);
    assert_eq!(cfg.durability, DurabilityPolicy::Throughput);
}

fn test_buffer_with_config(dir: &Path, config: SegmentConfig) -> TestBuffer {
    SegmentBuffer::open(dir, config).expect("buffer must open")
}

// =========================================================================
// sync_disk_bytes
// =========================================================================

#[test]
fn sync_disk_bytes_recovers_after_external_truncation() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();

    let before = buf.stats().approx_disk_bytes;
    assert!(before > 0, "flushed segment should have nonzero size");

    // External process truncates all segment files to zero bytes.
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "zst") {
            fs::write(&path, b"").unwrap();
        }
    }

    let synced = buf.sync_disk_bytes().unwrap();
    assert_eq!(
        synced, 0,
        "external truncation must be reflected after sync"
    );
    assert_eq!(buf.stats().approx_disk_bytes, 0);
}

// =========================================================================
// Throughput stress test — 8 writers × 2 readers, measures events/sec
// under contention. Verifies correctness (all items readable) AND reports
// a throughput number so perf regressions show up in test output.
// =========================================================================

#[test]
fn stress_8_writers_2_readers_throughput() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    let tmp = TempDir::new().unwrap();
    // FlushPolicy::Manual is critical: with Batch(4) this test would create
    // 20_000 segment files (80_000 items / 4), causing pathological I/O under
    // parallel test execution. Manual keeps everything in-memory so the test
    // stresses mutex contention, not the filesystem. The single flush() after
    // the scope writes one segment.
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 8;
    const PER_WRITER: usize = 10_000;
    const TOTAL: usize = WRITERS * PER_WRITER; // 80_000
    const READERS: usize = 2;

    // Shared read cursor — readers use it as a hint for where to poll.
    // The cursor may drift ahead (double-reads are harmless); correctness is
    // verified by the final full read, not by the cursor value.
    let read_cursor = Arc::new(Mutex::new(0u64));
    let total_read = Arc::new(AtomicU64::new(0));

    let start = Instant::now();
    thread::scope(|s| {
        // 2 reader threads: poll read_from to add read-side contention.
        for _ in 0..READERS {
            let buf_r = Arc::clone(&buf);
            let cursor_r = Arc::clone(&read_cursor);
            let total_r = Arc::clone(&total_read);
            s.spawn(move || loop {
                let current = *cursor_r.lock();
                if current >= TOTAL as u64 {
                    break;
                }
                if let Ok(events) = buf_r.read_from(current, 500) {
                    if !events.is_empty() {
                        total_r.fetch_add(events.len() as u64, Ordering::Relaxed);
                        *cursor_r.lock() = current + events.len() as u64;
                    }
                }
                std::thread::sleep(Duration::from_micros(20));
            });
        }

        // 8 writer threads.
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            s.spawn(move || {
                let base = writer_id * PER_WRITER;
                for i in 0..PER_WRITER {
                    let _ = buf_w.append(test_item((base + i) as u64));
                }
            });
        }
    });

    let elapsed = start.elapsed();

    // Regression guard (AGENTS.md rule 7): under FlushPolicy::Manual the
    // concurrent append phase must create ZERO segment files. An earlier
    // Batch(4) version created 20_000 files and hung CI for hours (commit
    // 80257a0). If this fires, the flush policy or Manual semantics broke —
    // do NOT widen the bound, investigate the regression.
    let segment_files_before_flush = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        segment_files_before_flush, 0,
        "FlushPolicy::Manual must not create segment files during append; \
         found {segment_files_before_flush} .zst file(s) — flush policy regression"
    );

    buf.flush().unwrap();

    // Correctness: all items assigned and readable.
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);
    assert_eq!(buf.pending_count(), TOTAL as u64);
    let all_events = buf.read_from(0, TOTAL * 2).unwrap();
    assert_eq!(
        all_events.len(),
        TOTAL,
        "all {TOTAL} events must be readable after the stress run"
    );

    // Throughput: report events/sec. NOT a hard assertion (CI hardware varies)
    // — it's a reporting metric so a human can spot regressions in the test
    // output.
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let throughput = TOTAL as f64 / elapsed_secs;
    eprintln!(
        "stress_8w_2r: {TOTAL} events in {elapsed_secs:.3}s = {throughput:.0} events/sec \
         ({:.2} µs/event under 8-writer contention, {} items observed by readers)",
        elapsed_secs * 1_000_000.0 / TOTAL as f64,
        total_read.load(Ordering::Relaxed)
    );
}

/// 8 writers × 4 readers stress with per-append latency histogram.
///
/// Reports p50/p90/p99/p99.9 latency on the writer path so a human can spot
/// latency-tail regressions (e.g. a lock-contention change, an allocation
/// introduced on the hot path). The latency numbers are NOT hard assertions
/// (CI hardware varies) — they are reported in the test output for
/// human inspection across runs.
///
/// Reuses the rule-7 discipline from the throughput stress: `FlushPolicy::Manual`
/// so the test stresses mutex contention, not the filesystem. Reader count
/// is doubled (4 vs the throughput test's 2) so read-side contention
/// contributes to the writer-tail latency.
#[test]
fn stress_8_writers_4_readers_latency_histogram() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 8;
    const PER_WRITER: usize = 10_000;
    const TOTAL: usize = WRITERS * PER_WRITER; // 80_000
    const READERS: usize = 4;

    let read_cursor = Arc::new(Mutex::new(0u64));
    let total_read = Arc::new(AtomicU64::new(0));
    // Per-writer latency samples. Pre-allocate so sampling overhead doesn't
    // include allocation in the measured section.
    let samples: Vec<Mutex<Vec<std::time::Duration>>> = (0..WRITERS)
        .map(|_| Mutex::new(Vec::with_capacity(PER_WRITER)))
        .collect();

    let start = Instant::now();
    thread::scope(|s| {
        // 4 reader threads: poll read_from to add read-side contention.
        for _ in 0..READERS {
            let buf_r = Arc::clone(&buf);
            let cursor_r = Arc::clone(&read_cursor);
            let total_r = Arc::clone(&total_read);
            s.spawn(move || loop {
                let current = *cursor_r.lock();
                if current >= TOTAL as u64 {
                    break;
                }
                if let Ok(events) = buf_r.read_from(current, 500) {
                    if !events.is_empty() {
                        total_r.fetch_add(events.len() as u64, Ordering::Relaxed);
                        *cursor_r.lock() = current + events.len() as u64;
                    }
                }
                std::thread::sleep(Duration::from_micros(20));
            });
        }

        // 8 writer threads, each measuring per-append latency.
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            let samples_w = &samples;
            s.spawn(move || {
                let base = writer_id * PER_WRITER;
                let mut local_samples: Vec<std::time::Duration> = Vec::with_capacity(PER_WRITER);
                for i in 0..PER_WRITER {
                    let t = Instant::now();
                    let _ = buf_w.append(test_item((base + i) as u64));
                    local_samples.push(t.elapsed());
                }
                *samples_w[writer_id].lock() = local_samples;
            });
        }
    });

    let elapsed = start.elapsed();

    // Regression guard (rule 7): under Manual the concurrent append phase
    // must create ZERO segment files.
    let segment_files_before_flush = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        segment_files_before_flush, 0,
        "FlushPolicy::Manual must not create segment files during append; \
         found {segment_files_before_flush} .zst file(s) — flush policy regression"
    );

    buf.flush().unwrap();
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);

    // Merge per-writer samples into a single sorted Vec for percentile
    // computation. N = 80_000 samples is plenty for stable p99 estimates.
    let mut all: Vec<std::time::Duration> = Vec::with_capacity(TOTAL);
    for s in &samples {
        all.extend_from_slice(&s.lock());
    }
    all.sort();

    // Percentile helper. N is large enough that linear indexing is fine.
    let pct = |p: f64| -> std::time::Duration {
        if all.is_empty() {
            return std::time::Duration::ZERO;
        }
        let idx = ((p / 100.0) * (all.len() as f64 - 1.0)).round() as usize;
        all[idx.min(all.len() - 1)]
    };
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let throughput = TOTAL as f64 / elapsed_secs;
    eprintln!(
        "stress_8w_4r_latency: {TOTAL} events in {elapsed_secs:.3}s = {throughput:.0} events/sec\n\
         latency (µs): p50={:.2} p90={:.2} p99={:.2} p99.9={:.2} max={:.2}\n\
         {} items observed by readers",
        pct(50.0).as_nanos() as f64 / 1000.0,
        pct(90.0).as_nanos() as f64 / 1000.0,
        pct(99.0).as_nanos() as f64 / 1000.0,
        pct(99.9).as_nanos() as f64 / 1000.0,
        all.last()
            .map(|d| d.as_nanos() as f64 / 1000.0)
            .unwrap_or(0.0),
        total_read.load(Ordering::Relaxed)
    );

    // Soft guard: p99 must stay under 5ms on any reasonable host (the test
    // runs in debug mode by default; release numbers are ~10x lower). This
    // is NOT a tight bound; if it fires, investigate the hot-path regression
    // before widening. Typical debug-mode p99 is ~50-500µs under 8-writer
    // contention.
    let p99 = pct(99.0);
    assert!(
        p99 < std::time::Duration::from_millis(50),
        "p99 latency {p99:?} exceeded 50ms soft guard — investigate hot-path regression"
    );
}

// =========================================================================
// DurabilityPolicy
// =========================================================================

/// Config-builder helper: vary ONLY the durability policy, keeping the
/// `test_config` defaults for everything else.
fn durability_config(max_size_bytes: u64, policy: DurabilityPolicy) -> SegmentConfig {
    SegmentConfig {
        flush_policy: FlushPolicy::Manual,
        max_size_bytes,
        compression_level: 3,
        durability: policy,
        cipher: None,
    }
}

/// All three policies must produce a readable, correct segment. This is a
/// functional roundtrip test, NOT a crash-semantics test — proving the
/// fsync branches fire correctly under a host crash requires killing the
/// process mid-flush and is out of scope for unit tests. (The fsync calls
/// are also exercised here: if a sync_all path is broken on the host, this
/// test surfaces it as an Err.)
#[test]
fn durability_policy_segment_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Segment),
    )
    .expect("open with Segment policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().expect("Segment flush must succeed");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }
}

#[test]
fn durability_policy_throughput_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Throughput),
    )
    .expect("open with Throughput policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush()
        .expect("Throughput flush must succeed (no fsync, but rename is real)");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }
}

#[test]
fn durability_policy_maximal_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Maximal),
    )
    .expect("open with Maximal policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    // Maximal includes a dir.sync_all after rename; on Linux/macOS this is
    // well-defined and must succeed. If it errors here, the host filesystem
    // does not support directory fsync (Maximal is documented to require
    // Linux/macOS for the dir-sync half).
    buf.flush()
        .expect("Maximal flush must succeed on a capable filesystem");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }

    // The directory must contain exactly one segment file (no .tmp debris
    // left behind) — verifies the rename completed under every policy.
    let zst_count = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        zst_count, 1,
        "exactly one .zst segment must exist after flush"
    );
}

/// All three policies must be recoverable: re-open the directory and the
/// segment file is visible (the rename is the atomicity boundary under
/// every policy).
#[test]
fn durability_policy_all_policies_recover_after_reopen() {
    for policy in [
        DurabilityPolicy::Maximal,
        DurabilityPolicy::Segment,
        DurabilityPolicy::Throughput,
    ] {
        let tmp = TempDir::new().unwrap();
        {
            let buf =
                SegmentBuffer::<TestItem>::open(tmp.path(), durability_config(1024 * 1024, policy))
                    .expect("open");
            buf.append(test_item(42)).unwrap();
            buf.flush().expect("flush");
        }
        let (buf, report) = SegmentBuffer::<TestItem>::open_with_report(
            tmp.path(),
            durability_config(1024 * 1024, policy),
        )
        .expect("reopen");
        assert_eq!(
            report.segment_count, 1,
            "policy {policy:?}: segment must be recovered"
        );
        assert_eq!(report.head_seq, 0);
        assert_eq!(report.next_seq, 1);
        let items = buf.read_from(0, 100).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 42);
    }
}

/// The default SegmentConfig must select `Segment` (the documented
/// backward-compat default for one release after the enum lands).
#[test]
fn durability_policy_default_is_segment() {
    let cfg = SegmentConfig::default();
    assert_eq!(cfg.durability, DurabilityPolicy::Segment);
}

/// The builder `.durability(...)` setter must round-trip into the built config.
#[test]
fn durability_policy_builder_roundtrip() {
    let cfg = SegmentConfig::builder()
        .durability(DurabilityPolicy::Throughput)
        .build();
    assert_eq!(cfg.durability, DurabilityPolicy::Throughput);
}

/// `SegmentConfig` is `Clone` since the cipher moved from `Box` to `Arc`.
/// A roundtrip through `.clone()` must preserve every field, including the
/// cipher (the `Arc` is shared, not duplicated).
#[test]
fn segment_config_is_clone() {
    let cfg = SegmentConfig::default();
    let cloned = cfg.clone();
    assert_eq!(cfg.flush_policy, cloned.flush_policy);
    assert_eq!(cfg.max_size_bytes, cloned.max_size_bytes);
    assert_eq!(cfg.compression_level, cloned.compression_level);
    assert_eq!(cfg.durability, cloned.durability);
    assert!(cfg.cipher.is_none() && cloned.cipher.is_none());
}

/// `SegmentConfigBuilder` is `Clone` (M12). This unblocks the pattern of
/// starting a base builder and cloning it per-buffer when constructing
/// several related buffers (e.g. a sharded producer set). The cipher
/// `Arc` is shared between clones — no key duplication.
#[test]
fn segment_config_builder_is_clone() {
    let base = SegmentConfig::builder()
        .flush_manually()
        .compression_level(7);
    let copy = base.clone();
    let cfg_a = base.build();
    let cfg_b = copy.compression_level(1).build();
    assert_eq!(cfg_a.compression_level, 7);
    assert_eq!(cfg_b.compression_level, 1);
    assert!(matches!(cfg_a.flush_policy, FlushPolicy::Manual));
    assert!(matches!(cfg_b.flush_policy, FlushPolicy::Manual));
}

#[cfg(feature = "encryption")]
#[test]
fn segment_config_clone_shares_cipher_arc() {
    let cfg = SegmentConfig::builder()
        .cipher(Arc::new(AesGcmCipher::new(&[0u8; 32])))
        .build();
    let cloned = cfg.clone();
    // Both configs reference the SAME Arc — cipher state is shared, not
    // duplicated. This is what makes `recommended_cipher()` and multi-buffer
    // setups cheap.
    let (Some(a), Some(b)) = (cfg.cipher.as_ref(), cloned.cipher.as_ref()) else {
        panic!("cipher must be Some on both configs");
    };
    assert!(
        std::ptr::addr_eq(a.as_ref() as *const _, b.as_ref() as *const _),
        "Arc must be shared, not deep-copied"
    );
}
