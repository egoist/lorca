//! XChaCha20-Poly1305 for account-key blobs; libsodium-compatible sealed boxes for anything
//! addressed to one public key (job envelopes, pairing, wrapped DEKs).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

use crate::keys::unb64_32;

const NONCE_LEN: usize = 24;
/// XChaCha20-Poly1305's nonce and authentication tag around the plaintext.
pub const ENVELOPE_OVERHEAD: usize = NONCE_LEN + 16;

/// What encrypts and opens an envelope: one key, or the account's keys, of which the newest
/// encrypts and any opens (`keys::AccountKeys`).
pub trait Keys {
    fn newest(&self) -> &[u8; 32];
    /// Every key that may open, newest first.
    fn each(&self) -> &[[u8; 32]];
}

impl Keys for [u8; 32] {
    fn newest(&self) -> &[u8; 32] {
        self
    }

    fn each(&self) -> &[[u8; 32]] {
        std::slice::from_ref(self)
    }
}

impl Keys for crate::keys::AccountKeys {
    fn newest(&self) -> &[u8; 32] {
        crate::keys::AccountKeys::newest(self)
    }

    fn each(&self) -> &[[u8; 32]] {
        crate::keys::AccountKeys::each(self)
    }
}

/// `nonce || ciphertext`, with the blob kind as associated data, under the newest key.
pub fn encrypt(keys: &(impl Keys + ?Sized), kind: &str, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(keys.newest().into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: kind.as_bytes() })
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Opens an envelope with whichever of the keys encrypted it.
pub fn decrypt(keys: &(impl Keys + ?Sized), kind: &str, envelope: &[u8]) -> anyhow::Result<Vec<u8>> {
    keys.each().iter().find_map(|key| open_with(key, kind, envelope)).ok_or_else(|| anyhow::anyhow!("decryption failed"))
}

/// Whether the newest of the keys encrypted the envelope.
pub fn under_newest(keys: &(impl Keys + ?Sized), kind: &str, envelope: &[u8]) -> bool {
    open_with(keys.newest(), kind, envelope).is_some()
}

fn open_with(key: &[u8; 32], kind: &str, envelope: &[u8]) -> Option<Vec<u8>> {
    if envelope.len() <= NONCE_LEN {
        return None;
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let (nonce, ciphertext) = envelope.split_at(NONCE_LEN);
    cipher.decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: kind.as_bytes() }).ok()
}

pub fn encrypt_json<T: serde::Serialize>(keys: &(impl Keys + ?Sized), kind: &str, value: &T) -> anyhow::Result<Vec<u8>> {
    encrypt(keys, kind, &serde_json::to_vec(value)?)
}

pub fn decrypt_json<T: serde::de::DeserializeOwned>(keys: &(impl Keys + ?Sized), kind: &str, envelope: &[u8]) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&decrypt(keys, kind, envelope)?)?)
}

/// Seal to a base64url X25519 public key.
pub fn seal(recipient_box_pubkey: &str, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let public = crypto_box::PublicKey::from_bytes(unb64_32(recipient_box_pubkey)?);
    public.seal(&mut rand::rngs::OsRng, plaintext).map_err(|_| anyhow::anyhow!("seal failed"))
}

pub fn seal_json<T: serde::Serialize>(recipient_box_pubkey: &str, value: &T) -> anyhow::Result<Vec<u8>> {
    seal(recipient_box_pubkey, &serde_json::to_vec(value)?)
}

pub fn unseal(secret: &crypto_box::SecretKey, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
    secret.unseal(ciphertext).map_err(|_| anyhow::anyhow!("unseal failed: not addressed to this key"))
}

pub fn unseal_json<T: serde::de::DeserializeOwned>(secret: &crypto_box::SecretKey, ciphertext: &[u8]) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&unseal(secret, ciphertext)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aead_round_trip_binds_kind() {
        let dek = crate::keys::random_32();
        let envelope = encrypt(&dek, "chat", b"hello").unwrap();
        assert_eq!(decrypt(&dek, "chat", &envelope).unwrap(), b"hello");
        assert!(decrypt(&dek, "roster", &envelope).is_err());
    }

    #[test]
    fn sealed_box_round_trip() {
        let machine = crate::keys::Machine::generate();
        let sealed = seal(&machine.box_pubkey(), b"job").unwrap();
        assert_eq!(unseal(&machine.box_secret, &sealed).unwrap(), b"job");
        let other = crate::keys::Machine::generate();
        assert!(unseal(&other.box_secret, &sealed).is_err());
    }

    /// Pinned vectors: the keys derived from a fixed machine secret, a signature, an envelope,
    /// and a sealed box. A change here is a change to every identity's keys.
    #[test]
    fn phone_port_vectors() {
        let machine = crate::keys::Machine::from_secret(crate::keys::unb64_32("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA").unwrap());
        assert_eq!(machine.pubkey(), "_sAIAgUy5PaCK26dHTqBF8Dc6c8e_uYar99JYvg5AmY");
        assert_eq!(machine.box_pubkey(), "sVOMlrkZwlTIVzxBw1dMM_8u4M4xe9fc50tRkF-W6VU");
        assert_eq!(crate::keys::identity_id(&machine.pubkey()), "64531081f468ed8d");
        let signature = crate::keys::unb64("ciNZNKBXgperVIZQSc3QZjQLsszNvtRctGscInX1fQfvAqa7hD3zpAJ3KyetWceTMhtX31fPbw8noPjA5uY4Cg").unwrap();
        let signature = ed25519_dalek::Signature::from_slice(&signature).unwrap();
        ed25519_dalek::Verifier::verify(&crate::keys::verifying_key(&machine.pubkey()).unwrap(), b"abc", &signature).unwrap();

        let dek = crate::keys::unb64_32("yMfGxcTDwsHAv769vLu6ubi3trW0s7KxsK-urayrqqk").unwrap();
        let envelope = crate::keys::unb64("YCLl5HooW12UVqhC4FvGBPGHxdZ1HlLcuJpi4z4e5vVejV1CXWlnSW6aFNC_5dDj6dSkBRgTnws7XwnW").unwrap();
        assert_eq!(decrypt(&dek, "chat", &envelope).unwrap(), b"hello from the phone");
        assert!(decrypt(&dek, "roster", &envelope).is_err());

        let sealed = crate::keys::unb64("RGYkCdfjJFQ26M8NEwucIRRhI96qSHwzi7j-MlbpiFIbveqwoBNsXgbXBSFiIXlueAQWyae6UCI5ql7T8zZkzz4G").unwrap();
        assert_eq!(unseal(&machine.box_secret, &sealed).unwrap(), b"job for the runner");
    }
}
