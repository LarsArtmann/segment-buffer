//! Fuzz target: opening a buffer over a directory full of arbitrary files must
//! not panic. This is the crash-recovery contract: recovery must survive any
//! on-disk garbage.
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_recovery
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
}

/// A single "file to drop in the directory" — a name fragment and bytes.
struct DirGarbage {
    entries: Vec<(String, Vec<u8>)>,
}

// libfuzzer works on raw bytes; interpret them as a sequence of (name, blob)
// pairs so the directory contains segment-like and non-segment files alike.
impl<'a> From<&'a [u8]> for DirGarbage {
    fn from(data: &'a [u8]) -> Self {
        let mut entries = Vec::new();
        let mut chunks = data.split(|b| *b == 0);
        while let (Some(name_chunk), rest) = (chunks.next(), {
            let mut peek = chunks.clone();
            peek.next()
        }) {
            let Some(blob) = chunks.next() else { break };
            // Only create files whose names look like segment files or are
            // short enough to be plausible directory entries.
            let name = String::from_utf8_lossy(name_chunk).into_owned();
            if !name.is_empty() && name.len() < 64 {
                entries.push((name, blob.to_vec()));
            }
            let _ = rest; // suppress unused
        }
        DirGarbage { entries }
    }
}

fuzz_target!(|data: &[u8]| {
    let garbage = DirGarbage::from(data);
    let Ok(dir) = tempfile::tempdir() else { return };

    for (name, bytes) in &garbage.entries {
        let _ = std::fs::write(dir.path().join(name), bytes);
    }

    // Opening over arbitrary directory contents must not panic.
    let _ = SegmentBuffer::<Item>::open(dir.path(), SegmentConfig::default());
});
