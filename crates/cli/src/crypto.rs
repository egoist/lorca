//! XChaCha20-Poly1305 for account-key blobs; libsodium-compatible sealed boxes for anything
//! addressed to one public key (job envelopes, pairing, wrapped DEKs).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

use crate::keys::unb64_32;

const NONCE_LEN: usize = 24;

/// `nonce || ciphertext`, with the blob kind as associated data.
pub fn encrypt(dek: &[u8; 32], kind: &str, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(dek.into());
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

pub fn decrypt(dek: &[u8; 32], kind: &str, envelope: &[u8]) -> anyhow::Result<Vec<u8>> {
    if envelope.len() <= NONCE_LEN {
        anyhow::bail!("envelope too short");
    }
    let cipher = XChaCha20Poly1305::new(dek.into());
    let (nonce, ciphertext) = envelope.split_at(NONCE_LEN);
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: kind.as_bytes() })
        .map_err(|_| anyhow::anyhow!("decryption failed"))
}

pub fn encrypt_json<T: serde::Serialize>(dek: &[u8; 32], kind: &str, value: &T) -> anyhow::Result<Vec<u8>> {
    encrypt(dek, kind, &serde_json::to_vec(value)?)
}

pub fn decrypt_json<T: serde::de::DeserializeOwned>(dek: &[u8; 32], kind: &str, envelope: &[u8]) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&decrypt(dek, kind, envelope)?)?)
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

    /// Vectors produced by the phone's TypeScript port (`mobile/src/core`). Both sides must keep
    /// deriving the same keys and opening each other's envelopes.
    #[test]
    fn phone_port_vectors() {
        let machine = crate::keys::Machine::from_secret(crate::keys::unb64_32("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA").unwrap());
        assert_eq!(machine.pubkey(), "KXfnobZE_H5jKjoU2vzdWTx9SYlxt-JNBapkeKQzUVo");
        assert_eq!(machine.box_pubkey(), "l4G7NMHlyHxj-9EQh-izsFceTPBys8oFzbLOJwhfJ08");
        assert_eq!(crate::keys::identity_id(&machine.pubkey()), "958263b260940c07");
        let signature = crate::keys::unb64("ivX9SkcoNZytd0MycLoFlmVgia78qGJjxEHbX78veJKN0Q7hcfegZoECSb37u-K4RrMsWQRd9Yspe4mqdRbXCw").unwrap();
        let signature = ed25519_dalek::Signature::from_slice(&signature).unwrap();
        ed25519_dalek::Verifier::verify(&crate::keys::verifying_key(&machine.pubkey()).unwrap(), b"abc", &signature).unwrap();

        let dek = crate::keys::unb64_32("yMfGxcTDwsHAv769vLu6ubi3trW0s7KxsK-urayrqqk").unwrap();
        let envelope = crate::keys::unb64("YCLl5HooW12UVqhC4FvGBPGHxdZ1HlLcuJpi4z4e5vVejV1CXWlnSW6aFNC_5dDj6dSkBRgTnws7XwnW").unwrap();
        assert_eq!(decrypt(&dek, "chat", &envelope).unwrap(), b"hello from the phone");
        assert!(decrypt(&dek, "roster", &envelope).is_err());

        let sealed = crate::keys::unb64("5GykOdJhJA8j4qUzTlAwPOLgkw4lM0TQOKeNt9iNUngUg5gbHFd_uzIfl3KpE3kIWPbYqBqxqto-Y4rhVykVkoqu").unwrap();
        assert_eq!(unseal(&machine.box_secret, &sealed).unwrap(), b"job for the runner");
    }
}
