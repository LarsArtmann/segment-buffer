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

/// Shared test config: small batch, auto-flush effectively disabled. Only
/// `max_size_bytes` varies between tests, so it is the single parameter.
fn test_config(max_size_bytes: u64) -> SegmentConfig {
    SegmentConfig {
        flush_policy: FlushPolicy::Batch(4),
        max_size_bytes,
        compression_level: 3,
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
    let mut inner = buf.inner.lock();
    inner.approx_disk_bytes = bytes;
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
            flush_policy: FlushPolicy::BatchOrInterval {
                batch_size: 256,
                interval: std::time::Duration::from_secs(1),
            },
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
    let payload = segment::encode_payload(Some(&cipher), 3, &path, &items).unwrap();
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
    // Io constructed from a bare io::Error via `?` has path = None.
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let err: SegmentError = io_err.into();
    let rendered = format!("{err}");
    // No " for <path>" clause when path is None.
    assert_eq!(rendered, "I/O error: missing");
}

#[test]
fn segment_error_io_display_format_with_path() {
    // Io constructed with explicit path context renders the path clause.
    let err = SegmentError::Io {
        path: Some(std::path::PathBuf::from(
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
