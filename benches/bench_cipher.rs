//! Benchmark: encryption overhead in the `flush()` encode pipeline.
//!
//! The encode pipeline is CBOR → zstd → **[cipher.encrypt]** → write_atomic.
//! This benchmark isolates the cipher cost by measuring the full `flush()`
//! path (which includes CBOR + zstd + disk I/O — constant across all variants)
//! against three configurations:
//!
//! 1. **No cipher** — the baseline (CBOR + zstd + write only).
//! 2. **AES-256-GCM** — the legacy cipher (12-byte nonce, AES-NI accelerated).
//! 3. **XChaCha20-Poly1305** — the recommended cipher (24-byte nonce, constant-time).
//!
//! Each iteration opens a fresh buffer, appends a batch of items in-memory
//! (`FlushPolicy::Manual`), then times a single `flush()` — the encode
//! pipeline runs exactly once per iteration.
//!
//! Run with:
//!   cargo bench --bench bench_cipher --features encryption

// Non-production code: unwrap/expect on setup failures, `as` conversions for
// batch sizes, and counter arithmetic are idiomatic and safe here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use segment_buffer::{
    AesGcmCipher, FlushPolicy, SegmentBuffer, SegmentCipher, SegmentConfig, XChaCha20Poly1305Cipher,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Items per flush — large enough that the cipher processes a realistic
/// payload, not just a few bytes.
const BATCH_SIZE: usize = 256;

#[derive(Serialize, Deserialize, Clone)]
struct Record {
    id: u64,
    payload: String,
}

fn make_items() -> Vec<Record> {
    (0..BATCH_SIZE)
        .map(|i| Record {
            id: i as u64,
            payload: format!("payload-{i}"),
        })
        .collect()
}

type CipherOpt = Option<Arc<dyn SegmentCipher + Send + Sync>>;

/// Open a buffer with the given cipher (or `None` for the no-cipher baseline),
/// append a full batch, and return it.
fn buffer_with_cipher(dir: &std::path::Path, cipher: CipherOpt) -> SegmentBuffer<Record> {
    let mut builder = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(u64::MAX)
        .compression_level(3);
    if let Some(c) = cipher {
        builder = builder.cipher(c);
    }
    let buf = SegmentBuffer::<Record>::open(dir, builder.build()).unwrap();
    for item in make_items() {
        buf.append(item).unwrap();
    }
    buf
}

fn bench_cipher(c: &mut Criterion) {
    let mut group = c.benchmark_group("cipher_flush");
    group.sample_size(30);

    let variants: [(BenchmarkId, CipherOpt); 3] = [
        (BenchmarkId::from_parameter("no_cipher"), None),
        (
            BenchmarkId::from_parameter("aes_256_gcm"),
            Some(Arc::new(AesGcmCipher::new(&[0x42u8; 32]))),
        ),
        (
            BenchmarkId::from_parameter("xchacha20_poly1305"),
            Some(Arc::new(XChaCha20Poly1305Cipher::new(&[0x42u8; 32]))),
        ),
    ];

    for (id, cipher) in variants {
        group.bench_with_input(id, &cipher, |b, cipher| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    let buf = buffer_with_cipher(tmp.path(), cipher.clone());
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    buf.flush().unwrap();
                },
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_cipher);
criterion_main!(benches);
