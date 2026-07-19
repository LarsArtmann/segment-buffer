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
use std::sync::Arc;

/// Helper trait that lets a `dyn Error + Send + Sync` trait object be upcast
/// to `&dyn Error` without requiring Rust 1.86's trait-upcasting coercion.
/// The crate's MSRV is 1.85; once it moves to 1.86+ this can be removed and
/// `source()` can be a plain `self.source.as_deref()`.
trait ErrorExt: fmt::Debug + std::error::Error {
    /// Upcast this error to a `dyn std::error::Error` reference.
    fn as_std_error(&self) -> &(dyn std::error::Error + 'static);
}

impl<T: std::error::Error + 'static> ErrorExt for T {
    fn as_std_error(&self) -> &(dyn std::error::Error + 'static) {
        self
    }
}

/// Error returned by [`SegmentCipher`] implementations.
///
/// Deliberately minimal: the cipher operates on bytes, not files, so it has no
/// path or sequence context to carry. The segment I/O layer enriches this into
/// a [`crate::SegmentError::Cipher`] with the offending file's path.
///
/// Construct with [`CipherError::msg`] for a plain message, or
/// [`CipherError::with_source`] when you want to preserve the underlying AEAD
/// (or other) error type for `std::error::Error::source()` chaining. The
/// fields are private so that adding context later is non-breaking.
#[derive(Debug, Clone)]
pub struct CipherError {
    /// Human-readable description of what went wrong.
    message: String,
    /// Optional underlying cause (e.g. the AEAD crate's opaque error).
    /// `Arc` (not `Box`) so [`CipherError`] stays [`Clone`]. Surfaced via
    /// [`std::error::Error::source`].
    source: Option<Arc<dyn ErrorExt + Send + Sync>>,
}

impl CipherError {
    /// Construct a [`CipherError`] from anything displayable, with no
    /// underlying cause.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::CipherError;
    ///
    /// let err = CipherError::msg("key not configured");
    /// assert_eq!(err.to_string(), "key not configured");
    /// assert!(std::error::Error::source(&err).is_none());
    /// ```
    pub fn msg(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
            source: None,
        }
    }

    /// Construct a [`CipherError`] that preserves the underlying error so
    /// operators can inspect it via [`std::error::Error::source`].
    ///
    /// Use this when wrapping a typed error from an AEAD implementation
    /// (`aes_gcm::Error`, `chacha20poly1305::Error`, …) so the original
    /// failure is not erased behind a `format!`.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::CipherError;
    /// use std::fmt;
    /// use std::error::Error;
    ///
    /// /// A tiny typed error an AEAD crate might expose.
    /// #[derive(Debug)]
    /// struct AeadError;
    /// impl fmt::Display for AeadError {
    ///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    ///         f.write_str("tag mismatch")
    ///     }
    /// }
    /// impl std::error::Error for AeadError {}
    ///
    /// let err = CipherError::with_source("AES-GCM decryption failed", AeadError);
    /// assert_eq!(err.to_string(), "AES-GCM decryption failed");
    /// // The underlying cause is preserved via `source()`:
    /// let src = err.source().expect("source should be set by with_source");
    /// assert_eq!(src.to_string(), "tag mismatch");
    /// ```
    pub fn with_source<E>(message: impl fmt::Display, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            message: message.to_string(),
            source: Some(Arc::new(source)),
        }
    }
}

impl fmt::Display for CipherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CipherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // `Arc<dyn ErrorExt + Send + Sync>` cannot be coerced to
        // `&dyn Error` until trait-upcasting stabilises for our MSRV (1.86+);
        // `ErrorExt::as_std_error` does the upcast explicitly.
        self.source.as_ref().map(|s| s.as_std_error())
    }
}

/// Encrypts and decrypts segment file payloads.
///
/// Implementations must be [`Send`] + [`Sync`] because the buffer is shared
/// across threads via `Arc<SegmentBuffer>`.
///
/// The ciphertext format is implementation-defined but must be self-describing:
/// [`decrypt`](Self::decrypt) must be able to recover the plaintext from the
/// exact bytes returned by [`encrypt`](Self::encrypt) without external state.
///
/// # Naming
///
/// The trait is called `SegmentCipher`, not `SegmentAead`, even though the
/// shipped implementation ([`AesGcmCipher`]) is an AEAD. This is deliberate:
/// the trait contract is "any stateless self-describing encrypt/decrypt pair",
/// which admits AEADs (recommended), HMAC-wrapped symmetric ciphers, or even
/// custom schemes that combine encryption with a separate authenticator.
/// Renaming to `SegmentAead` would narrow the contract to AEADs only — a
/// constraint the trait does not actually enforce. Use an AEAD in practice;
/// the trait stays general on purpose.
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
    use std::fmt;
    use std::sync::Arc;

    /// Wrapper that turns any `Display`able AEAD error (e.g. the opaque
    /// `aes_gcm::Error`, which intentionally does not impl `std::error::Error`)
    /// into something that does, so it can flow through
    /// [`std::error::Error::source`] chains without losing the original
    /// diagnostic message.
    #[derive(Debug, Clone)]
    struct AeadError(String);

    impl fmt::Display for AeadError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for AeadError {}

    fn wrap<E: fmt::Display>(message: &'static str, e: E) -> CipherError {
        CipherError {
            message: message.to_string(),
            source: Some(Arc::new(AeadError(e.to_string()))),
        }
    }

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
        /// # Example
        ///
        /// ```
        /// use segment_buffer::AesGcmCipher;
        ///
        /// let key = [0u8; 32];
        /// let _cipher = AesGcmCipher::from_slice(&key).unwrap();
        /// ```
        ///
        /// # Errors
        ///
        /// Returns [`CipherError`] if the key length is not 32 bytes.
        pub fn from_slice(key_bytes: &[u8]) -> Result<Self, CipherError> {
            use aes_gcm::KeyInit;
            let cipher = aes_gcm::Aes256Gcm::new_from_slice(key_bytes)
                .map_err(|e| wrap("invalid AES-256 key", e))?;
            Ok(Self { cipher })
        }

        /// Create a new cipher from a 32-byte AES-256 key (const-sized input).
        ///
        /// # Example
        ///
        /// ```
        /// use segment_buffer::AesGcmCipher;
        /// use segment_buffer::SegmentCipher;
        ///
        /// let cipher = AesGcmCipher::new(&[0u8; 32]);
        /// let ciphertext = cipher.encrypt(b"hello").unwrap();
        /// let plaintext = cipher.decrypt(&ciphertext).unwrap();
        /// assert_eq!(plaintext, b"hello");
        /// ```
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
                .map_err(|e| wrap("AES-GCM encryption failed", e))?;

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
                .map_err(|e| wrap("AES-GCM decryption failed", e))
        }
    }
}

#[cfg(feature = "encryption")]
pub use private::AesGcmCipher;
