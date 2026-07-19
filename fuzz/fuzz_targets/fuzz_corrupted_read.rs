//! Fuzz target: reading a segment file whose bytes have been corrupted must
//! return `Err`, never panic.
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_corrupted_read
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
}

fuzz_target!(|corruption: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else { return };
    let Ok(buf) = SegmentBuffer::<Item>::open(dir.path(), SegmentConfig::default()) else {
        return;
    };

    // Write one valid item so a segment file exists on disk.
    if buf.append(Item { id: 0 }).is_err() {
        return;
    }
    if buf.flush().is_err() {
        return;
    }

    // Overwrite the segment file with arbitrary bytes (the corruption payload).
    if let Ok(entries) = std::fs::read_dir(dir.path()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "zst") {
                let _ = std::fs::write(&path, corruption);
            }
        }
    }

    // Reading the corrupted segment must not panic; an `Err` is expected.
    let _ = buf.read_from(0, 100);
});
