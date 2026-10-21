use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::cell::Cell;

use crate::{Error, Result};

const MAGIC: &[u8; 8] = b"ORBIT001";
const NONCE_BYTES: usize = 24;
pub(crate) const OVERHEAD: usize = MAGIC.len() + NONCE_BYTES + 16;

pub(crate) struct Cipher {
    inner: XChaCha20Poly1305,
    decryptions: Cell<u64>,
}

impl Cipher {
    pub(crate) fn new(key: &[u8; 32]) -> Self {
        Self {
            inner: XChaCha20Poly1305::new(key.into()),
            decryptions: Cell::new(0),
        }
    }

    pub(crate) fn decryption_count(&self) -> u64 {
        self.decryptions.get()
    }

    pub(crate) fn encrypt(&self, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_BYTES];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| Error::Random)?;
        let ciphertext = self
            .inner
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: context,
                },
            )
            .map_err(|_| Error::Authentication)?;
        let mut envelope = Vec::with_capacity(OVERHEAD + plaintext.len());
        envelope.extend_from_slice(MAGIC);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    pub(crate) fn decrypt(&self, context: &[u8], envelope: &[u8]) -> Result<Vec<u8>> {
        if envelope.len() < OVERHEAD || !envelope.starts_with(MAGIC) {
            return Err(Error::Authentication);
        }
        let plaintext = self
            .inner
            .decrypt(
                XNonce::from_slice(&envelope[MAGIC.len()..MAGIC.len() + NONCE_BYTES]),
                Payload {
                    msg: &envelope[MAGIC.len() + NONCE_BYTES..],
                    aad: context,
                },
            )
            .map_err(|_| Error::Authentication)?;
        self.decryptions.set(self.decryptions.get() + 1);
        Ok(plaintext)
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!("{hash:x}")
}
