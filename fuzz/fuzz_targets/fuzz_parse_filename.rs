//! Fuzz target: `parse_filename` over arbitrary UTF-8 must never panic, and
//! every accepted parse must be reproducible (the canonical filename of a
//! parsed range must parse back to the same range). This is the load-bearing
//! crash-recovery contract.
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_parse_filename
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::fuzz_hooks::{filename, parse_filename};

fuzz_target!(|input: &str| {
    // Contract 1: parse_filename must never panic on any &str input.
    let Some(parsed) = parse_filename(input) else {
        return;
    };

    // Contract 2: the canonical filename of a parsed range must parse back
    // to the same range. Catches normalization drift (e.g. a format string
    // that changes padding).
    let canonical = filename(parsed.start, parsed.end);
    let reparsed = parse_filename(&canonical).expect("canonical must reparse");
    assert_eq!(reparsed.start, parsed.start, "start must be stable");
    assert_eq!(reparsed.end, parsed.end, "end must be stable");
});
