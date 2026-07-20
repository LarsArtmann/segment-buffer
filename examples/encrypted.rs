//! Encrypted segment-buffer: segments are encrypted at rest with AES-256-GCM.
//!
//! Run with: `cargo run --example encrypted --features encryption`

use segment_buffer::{AesGcmCipher, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct SecretRecord {
    id: u64,
    payload: String,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // In production, load this from a secure key store (file, KMS, env var).
    let key = [0x42u8; 32];

    let tmp = tempfile::tempdir()?;

    let config = SegmentConfig::builder()
        .flush_at_batch_or_interval(256, std::time::Duration::from_secs(5))
        .max_size_bytes(1024 * 1024)
        .compression_level(3)
        .cipher(Arc::new(AesGcmCipher::new(&key)))
        .build();

    let buffer = SegmentBuffer::<SecretRecord>::open(tmp.path(), config)?;

    let record = SecretRecord {
        id: 1,
        payload: "classified".into(),
    };
    buffer.append(record.clone())?;
    buffer.flush()?;

    // The on-disk file is NOT plaintext — it's nonce + AES-GCM ciphertext.
    let entries = std::fs::read_dir(tmp.path())?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "zst") {
            let bytes = std::fs::read(&path)?;
            println!(
                "Segment {:?}: {} bytes (encrypted)",
                path.file_name(),
                bytes.len()
            );
            assert!(!bytes.windows(8).any(|w| w == b"classified"));
        }
    }

    // But we can still read it back with the key.
    let recovered = buffer.read_from(0, 100)?;
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0], record);
    println!("Decrypted {} record successfully", recovered.len());

    Ok(())
}
