//! Bring-your-own cipher: ChaCha20-Poly1305 as a `SegmentCipher` impl.
//!
//! This is the runnable counterpart to the snippet in `docs/CIPHERS.md`. It
//! lives here (rather than as a markdown doctest) so `cargo test --features
//! encryption --doc` / `cargo build --examples --features encryption` catches
//! any future API drift in the `chacha20poly1305` or `rand` crates against
//! the trait contract.
//!
//! See `docs/CIPHERS.md` → "Bring-your-own: ChaCha20-Poly1305" for the prose.

use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use rand::Rng;
use segment_buffer::{CipherError, SegmentCipher};
use std::fmt;

/// A bring-your-own ChaCha20-Poly1305 cipher wrapping the `chacha20poly1305` crate.
pub struct ChaChaCipher(ChaCha20Poly1305);

impl ChaChaCipher {
    /// Construct from a 32-byte key.
    pub fn new(key: &[u8; 32]) -> Self {
        Self(ChaCha20Poly1305::new(key.into()))
    }
}

impl SegmentCipher for ChaChaCipher {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError> {
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .0
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: b"",
                },
            )
            .map_err(|e| CipherError::msg(format!("chacha20poly1305 encrypt: {e}")))?;
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

fn main() {
    let cipher = ChaChaCipher::new(&[0u8; 32]);
    let plaintext = b"hello, cloud-sync drain loop";
    let ciphertext = cipher.encrypt(plaintext).expect("encrypt");
    let recovered = cipher.decrypt(&ciphertext).expect("decrypt");
    assert_eq!(recovered, plaintext);
    println!(
        "ChaCha20-Poly1305 bring-your-own roundtrip OK ({} bytes)",
        plaintext.len()
    );
}
