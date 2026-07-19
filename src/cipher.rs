//! Pluggable encryption for segment files at rest.
//!
//! The [`SegmentCipher`] trait abstracts the encrypt/decrypt operations so
//! callers can bring any AEAD (AES-GCM, ChaCha20-Poly1305, etc.). When the
//! `encryption` feature is enabled, a ready-made [`AesGcmCipher`] is provided.
//!
//! Cipher implementations return the lightweight [`CipherError`] so they don't
//! need to know about segment paths or the wider [`crate::SegmentError`]
//! hierarchy. The segment I/O layer attaches path context when promoting a
//! [`CipherError`] to a [`crate::SegmentError::Cipher`].

use std::fmt;

/// Error returned by [`SegmentCipher`] implementations.
///
/// Deliberately minimal: the cipher operates on bytes, not files, so it has no
/// path or sequence context to carry. The segment I/O layer enriches this into
/// a [`crate::SegmentError::Cipher`] with the offending file's path.
#[derive(Debug, Clone)]
pub struct CipherError(pub String);

impl CipherError {
    /// Construct a [`CipherError`] from anything displayable.
    pub fn msg(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }
}

impl fmt::Display for CipherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CipherError {}

/// Encrypts and decrypts segment file payloads.
///
/// Implementations must be [`Send`] + [`Sync`] because the buffer is shared
/// across threads via `Arc<SegmentBuffer>`.
///
/// The ciphertext format is implementation-defined but must be self-describing:
/// [`decrypt`](Self::decrypt) must be able to recover the plaintext from the
/// exact bytes returned by [`encrypt`](Self::encrypt) without external state.
///
/// # Example
///
/// ```
/// use segment_buffer::{CipherError, SegmentCipher};
///
/// struct Rot13;
///
/// impl SegmentCipher for Rot13 {
///     fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError> {
///         Ok(plaintext.iter().map(|b| b.wrapping_add(13)).collect())
///     }
///     fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError> {
///         Ok(ciphertext.iter().map(|b| b.wrapping_sub(13)).collect())
///     }
/// }
/// ```
pub trait SegmentCipher: Send + Sync {
    /// Encrypt `plaintext`, returning self-describing ciphertext.
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError>;

    /// Decrypt previously-produced ciphertext back to the original plaintext.
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError>;
}

// ---------------------------------------------------------------------------
// AES-256-GCM implementation (behind the `encryption` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "encryption")]
mod private {
    use super::{CipherError, SegmentCipher};

    /// AES-256-GCM cipher with a random 12-byte nonce prepended to each ciphertext.
    ///
    /// The on-disk payload format is: `[12-byte nonce][ciphertext + 16-byte GCM tag]`.
    /// This is byte-compatible with monitor365's `EncryptionKey` segment format,
    /// so existing encrypted segments can be read without migration. (The segment
    /// file envelope, if present, is stripped before the cipher sees the bytes.)
    pub struct AesGcmCipher {
        cipher: aes_gcm::Aes256Gcm,
    }

    impl AesGcmCipher {
        /// Create a new cipher from a 32-byte AES-256 key.
        ///
        /// # Errors
        ///
        /// Returns [`CipherError`] if the key length is not 32 bytes.
        pub fn from_slice(key_bytes: &[u8]) -> Result<Self, CipherError> {
            use aes_gcm::KeyInit;
            let cipher = aes_gcm::Aes256Gcm::new_from_slice(key_bytes)
                .map_err(|e| CipherError::msg(format!("invalid AES-256 key: {e}")))?;
            Ok(Self { cipher })
        }

        /// Create a new cipher from a 32-byte AES-256 key (const-sized input).
        pub fn new(key_bytes: &[u8; 32]) -> Self {
            use aes_gcm::KeyInit;
            Self {
                cipher: aes_gcm::Aes256Gcm::new_from_slice(key_bytes)
                    .expect("32-byte key is always valid for AES-256"),
            }
        }
    }

    impl SegmentCipher for AesGcmCipher {
        fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CipherError> {
            use aes_gcm::aead::Aead;
            use rand::RngCore;

            let mut nonce_bytes = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut nonce_bytes);
            let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

            let ciphertext = self
                .cipher
                .encrypt(nonce, plaintext)
                .map_err(|e| CipherError::msg(format!("AES-GCM encryption: {e}")))?;

            let mut out = Vec::with_capacity(12 + ciphertext.len());
            out.extend_from_slice(&nonce_bytes);
            out.extend_from_slice(&ciphertext);
            Ok(out)
        }

        fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CipherError> {
            use aes_gcm::aead::Aead;

            if ciphertext.len() < 12 {
                return Err(CipherError::msg("ciphertext too small for nonce prefix"));
            }
            let (nonce_bytes, encrypted) = ciphertext.split_at(12);
            let nonce = aes_gcm::Nonce::from_slice(nonce_bytes);

            self.cipher
                .decrypt(nonce, encrypted)
                .map_err(|e| CipherError::msg(format!("AES-GCM decryption: {e}")))
        }
    }
}

#[cfg(feature = "encryption")]
pub use private::AesGcmCipher;
