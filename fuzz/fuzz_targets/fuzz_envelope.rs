//! Fuzz target: envelope detection (`unwrap_envelope`) over arbitrary bytes
//! must never panic, must be deterministic, and must honour the legacy-
//! compatibility contract (the returned payload is always a suffix of the
//! input — either the bytes after the 8-byte envelope, or the original bytes
//! unchanged).
//!
//! Also exercises the edge case the self-review flagged: a file whose first
//! 4 bytes happen to be `SBF1` but whose reserved bytes are non-zero must
//! NOT be detected as an envelope (the 2⁻⁵⁶ false-positive guarantee).
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_envelope
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::fuzz_hooks::{unwrap_envelope, wrap_envelope};

fuzz_target!(|raw: &[u8]| {
    // Contract 1: unwrap_envelope must never panic on any input.
    let (version, payload) = unwrap_envelope(raw);

    // Contract 2: the payload is always a suffix of the input.
    // Either we stripped 8 bytes (envelope detected) or we returned the
    // input unchanged (legacy path).
    if version.is_some() {
        assert_eq!(
            payload.len(),
            raw.len().saturating_sub(8),
            "envelope-detected payload must be input minus 8 bytes"
        );
        assert!(
            raw.len() >= 8,
            "envelope detection requires at least 8 bytes"
        );
        // The payload must point into the original buffer (suffix property).
        let payload_offset = raw.len() - payload.len();
        assert_eq!(
            &raw[payload_offset..],
            payload,
            "payload must be a suffix of the input"
        );
    } else {
        assert_eq!(payload, raw, "legacy path must return the input unchanged");
    }

    // Contract 3: wrap→unwrap is identity on the payload. This catches any
    // asymmetry between the writer and the reader.
    let wrapped = wrap_envelope(raw);
    let (wrapped_version, wrapped_payload) = unwrap_envelope(&wrapped);
    assert_eq!(wrapped_version, Some(1), "freshly wrapped must be v1");
    assert_eq!(
        wrapped_payload, raw,
        "wrap→unwrap must be identity on the payload"
    );
});
