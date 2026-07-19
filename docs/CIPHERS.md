# Ciphers: AES-GCM and bring-your-own AEAD

`segment-buffer` ships with a single built-in cipher (`AesGcmCipher`, behind
the `encryption` Cargo feature) and a trait (`SegmentCipher`) that lets you
plug in any stateless AEAD or symmetric scheme. This doc covers:

1. The built-in AES-256-GCM cipher and its on-disk format.
2. The `SegmentCipher` contract and what it requires.
3. Worked examples for two common "bring your own" cases: ChaCha20-Poly1305
   (via the `chacha20poly1305` crate) and a no-op cipher for testing.

> See also: [`docs/DOMAIN_LANGUAGE.md` → `SegmentCipher`](./DOMAIN_LANGUAGE.md#segmentcipher)
> for the trait's role in the architecture, and `examples/encrypted.rs` for a
> runnable AES-GCM end-to-end example.

## The built-in: AES-256-GCM

### On-disk format

`AesGcmCipher` writes the following into the segment payload (after the
8-byte `SBF1` envelope is stripped on read, before it is prepended on write):

```text
[ 12-byte nonce ][ ciphertext + 16-byte GCM tag ]
```

- **Nonce**: 12 bytes, freshly random per `encrypt` call (uses `rand::rngs::OsRng`).
  The nonce is not secret; it is stored in plaintext at the head of the
  payload so `decrypt` can recover it.
- **Ciphertext + tag**: AES-256-GCM over the plaintext bytes, with an empty
  AAD (additional authenticated data). The 16-byte GCM tag is appended by
  `aes-gcm` and authenticated with the ciphertext.

This layout is byte-compatible with the original monitor365 cipher format,
so existing encrypted segment files read without migration.

### Usage

```rust
use segment_buffer::{AesGcmCipher, SegmentBuffer, SegmentConfig};

let key = [0u8; 32]; // 32-byte AES-256 key; generate via a KDF in production
let cipher = AesGcmCipher::new(&key);

let mut config = SegmentConfig::default();
config.cipher = Some(Box::new(cipher));

let buf = SegmentBuffer::<MyItem>::open("/tmp/encrypted-queue", config)?;
```

### Key management

`AesGcmCipher::new(key)` takes a 32-byte key. The cipher is `Clone` (it
holds an `Arc<Aes256>` internally), so the same cipher can be reused across
multiple buffers if you want them to share an encryption domain. **Do not**
rotate the key while segments are in flight — existing files are encrypted
under the key they were written with, and the cipher has no notion of key
versioning. For key rotation, migrate by reading with the old key, writing
with the new key to a new directory, and atomically swapping.

## The `SegmentCipher` trait

```rust
pub trait SegmentCipher: Send + Sync + Debug {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError>;
}
```

### Contract

1. **`encrypt ∘ decrypt` is identity on the plaintext.** Always. If
   `encrypt(p)` returns `c`, then `decrypt(c)` must return `Ok(p)`. The
   buffer's read path does nottolerate asymmetry.

2. **Stateless and self-describing.** `decrypt` must recover the plaintext
   from the exact bytes returned by `encrypt`, with no external state. This
   means nonces, salts, or other metadata must be embedded in the ciphertext
   output. (The built-in cipher embeds the 12-byte nonce; your cipher must
   embed whatever it needs.)

3. **`Send + Sync`.** The cipher is stored inside `SegmentConfig`, which is
   held by `SegmentBuffer` and accessed from any thread that touches the
   buffer. If your cipher wraps a non-`Sync` primitive, wrap it in a
   `Mutex` or convert to an `Arc<...>`.

4. **`Debug`.** Required so `SegmentConfig` can derive `Debug`. **Do not
   emit the key** in your `Debug` impl — see the `AesGcmCipher` source for
   the redaction pattern (`AesGcmCipher { .. }` with no key bytes).

### Authentication

AEADs (recommended) bundle confidentiality + authentication. The trait does
not *require* an AEAD — a plain symmetric cipher without authentication
would technically satisfy the type signature. **Don't do this.** The
segment file is the trust boundary: without authentication, a tampered
segment can cause arbitrary CBOR decode failures (best case) or silent
data corruption (worst case). The trait is named `SegmentCipher`, not
`SegmentAead`, only to admit custom schemes that combine a symmetric cipher
with a separate authenticator (e.g., XChaCha20 + Poly1305 composed
manually). Use an AEAD in practice.

## Bring-your-own: ChaCha20-Poly1305

Drop-in replacement for AES-GCM using the `chacha20poly1305` crate. Same
trait, same on-disk shape (`[12-byte nonce][ciphertext + 16-byte tag]`).

```toml
# Cargo.toml
[dependencies]
segment-buffer = { version = "0.4", features = ["encryption"] }
chacha20poly1305 = "0.10"
rand = "0.8"
```

```rust
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::{Aead, Payload}};
use rand::rngs::OsRng;
use rand::RngCore;
use segment_buffer::{SegmentCipher, CipherError};
use std::fmt;

pub struct ChaChaCipher(ChaCha20Poly1305);

impl ChaChaCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self(ChaCha20Poly1305::new(key.into()))
    }
}

impl SegmentCipher for ChaChaCipher {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self.0
            .encrypt(nonce, Payload { msg: plaintext, aad: b"" })
            .map_err(|e| CipherError::msg(format!("chacha20poly1305 encrypt: {e}")))?;
        // Prepend the nonce so decrypt can recover it.
        Ok(nonce_bytes.into_iter().chain(ciphertext).collect())
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError> {
        if ciphertext.len() < 12 {
            return Err(CipherError::msg("ciphertext too short for nonce"));
        }
        let (nonce_bytes, ct) = ciphertext.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.0
            .decrypt(nonce, Payload { msg: ct, aad: b"" })
            .map_err(|e| CipherError::msg(format!("chacha20poly1305 decrypt: {e}")))
    }
}

impl fmt::Debug for ChaChaCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChaChaCipher").finish_non_exhaustive()
    }
}
```

Wire it in exactly like the built-in:

```rust
let cipher = ChaChaCipher::new(&key);
let mut config = SegmentConfig::default();
config.cipher = Some(Box::new(cipher));
let buf = SegmentBuffer::<MyItem>::open(dir, config)?;
```

### Why prefer ChaCha20-Poly1305?

- **No AES-NI dependency.** On platforms without hardware AES (some ARM
  chips, embedded targets, debug builds without target-features), ChaCha20
  is significantly faster.
- **Constant-time by construction.** ChaCha20 is a software ARX cipher; it
  does not rely on lookup tables that vary by cache state.
- **XChaCha20-Poly1305 variant** (24-byte nonce, available in the same
  crate) allows random-nonce generation without collision risk across
  essentially unbounded write counts — useful if you ever expose
  user-supplied plaintexts that might be repeated.

## Bring-your-own: no-op cipher (testing only)

For tests that need the cipher plumbing exercised without actual encryption
overhead:

```rust
use segment_buffer::{SegmentCipher, CipherError};
use std::fmt;

#[derive(Debug)]
pub struct NoOpCipher;

impl SegmentCipher for NoOpCipher {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(plaintext.to_vec())
    }
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(ciphertext.to_vec())
    }
}
```

**Never use this in production.** It provides zero confidentiality and zero
authentication. It exists only to let tests exercise the cipher code path
without paying for real crypto.

## What the cipher does NOT see

The cipher is applied **after** the segment payload is constructed and
**before** the `SBF1` envelope is prepended. Concretely:

```
[ items: Vec<T> ]
       │
       ▼ CBOR-serialize
[ CBOR bytes ]
       │
       ▼ zstd-compress
[ zstd bytes ]
       │
       ▼ cipher.encrypt         ← you are here
[ encrypted zstd-CBOR bytes ]
       │
       ▼ prepend SBF1 envelope
[ 8-byte envelope + encrypted payload ]
       │
       ▼ atomic write + rename
[ seg_*.zst file on disk ]
```

Implications:

- **The cipher does not see item boundaries** — it operates on the
  compressed CBOR blob. A 1-byte change to any item propagates through CBOR
  + zstd and produces a completely different ciphertext byte stream.
- **The cipher does not see the filename** — `seg_{start:012}_{end:012}.zst`
  is on disk in plaintext. This is deliberate (filename-based recovery
  requires it). If the sequence-number metadata is sensitive in your
  deployment, store the buffer inside an encrypted volume; the crate will
  not encrypt filenames.
- **The cipher does not see the envelope** — the `SBF1` magic, version,
  and reserved bytes are prepended/stripped by the segment format code,
  outside the cipher's responsibility.

## Performance notes

Encryption cost is **amortized per flush, not per append.** A flush
encrypts one zstd-CBOR blob of `len(unflushed)` items; the per-item
encryption cost is negligible compared to CBOR serialize + zstd compress +
disk fsync. Benchmarking ChaCha20 vs AES-GCM at the segment granularity is
dominated by I/O, not by the cipher — choose based on platform and threat
model, not microbenchmarks.

The built-in `AesGcmCipher` uses `aes-gcm` 0.10 which auto-selects AES-NI
on x86-64 with `target-feature = "aes"`. On other targets it falls back to
a constant-time software implementation. Verify your `RUSTFLAGS` /
`target-cpu` settings if encryption throughput matters in your deployment.
