//! Identity and machine keys.
//!
//! Master secret (32 bytes) → HKDF → identity signing key (Ed25519) and content box key
//! (X25519). Each Device has its own 32-byte machine secret → HKDF → machine signing key and
//! machine box key. Public keys travel as base64url; the identity id is `sha256(pubkey)`.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn unb64(text: &str) -> anyhow::Result<Vec<u8>> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('='))?)
}

pub fn unb64_32(text: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = unb64(text)?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("expected 32 bytes"))
}

fn derive(secret: &[u8; 32], info: &str) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(b"tinybot-v1"), secret);
    let mut out = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut out).expect("hkdf expand");
    out
}

pub fn random_32() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Backup phrase: base32 (lowercase, no padding) of the master secret, in groups of four.
pub fn phrase_from_secret(secret: &[u8; 32]) -> Vec<String> {
    let encoded = data_encoding::BASE32_NOPAD.encode(secret).to_lowercase();
    encoded.as_bytes().chunks(4).map(|chunk| String::from_utf8_lossy(chunk).into_owned()).collect()
}

pub fn secret_from_phrase(phrase: &str) -> anyhow::Result<[u8; 32]> {
    let cleaned: String = phrase.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
    let bytes = data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .map_err(|_| anyhow::anyhow!("That does not look like a Tinybot backup phrase"))?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("Backup phrase has the wrong length"))
}

/// Keys derived from the master secret. Only the identity device holds this.
pub struct Identity {
    pub master: [u8; 32],
    pub signing: SigningKey,
    pub content_secret: crypto_box::SecretKey,
}

impl Identity {
    pub fn from_master(master: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&derive(&master, "identity/sign"));
        let content_secret = crypto_box::SecretKey::from_bytes(derive(&master, "identity/content-box"));
        Identity { master, signing, content_secret }
    }

    pub fn generate() -> Self {
        Self::from_master(random_32())
    }

    pub fn pubkey(&self) -> String {
        b64(self.signing.verifying_key().as_bytes())
    }

    pub fn content_pubkey(&self) -> String {
        b64(self.content_secret.public_key().as_bytes())
    }

    /// `hash(pubkey)`, hex, shortened for display.
    #[allow(dead_code)]
    pub fn id(&self) -> String {
        identity_id(&self.pubkey())
    }

    pub fn phrase(&self) -> Vec<String> {
        phrase_from_secret(&self.master)
    }

    pub fn sign(&self, message: &[u8]) -> String {
        b64(&self.signing.sign(message).to_bytes())
    }
}

pub fn identity_id(pubkey: &str) -> String {
    let digest = Sha256::digest(pubkey.as_bytes());
    hex(&digest[..8])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Keys for this Device.
pub struct Machine {
    pub secret: [u8; 32],
    pub signing: SigningKey,
    pub box_secret: crypto_box::SecretKey,
}

impl Machine {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&derive(&secret, "machine/sign"));
        let box_secret = crypto_box::SecretKey::from_bytes(derive(&secret, "machine/box"));
        Machine { secret, signing, box_secret }
    }

    pub fn generate() -> Self {
        Self::from_secret(random_32())
    }

    pub fn pubkey(&self) -> String {
        b64(self.signing.verifying_key().as_bytes())
    }

    pub fn box_pubkey(&self) -> String {
        b64(self.box_secret.public_key().as_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> String {
        b64(&self.signing.sign(message).to_bytes())
    }
}

pub fn verifying_key(pubkey: &str) -> anyhow::Result<VerifyingKey> {
    Ok(VerifyingKey::from_bytes(&unb64_32(pubkey)?)?)
}

/// `identity.json`: the master secret. Present only on identity devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityFile {
    pub master_secret: String,
    pub created_at: i64,
}

impl IdentityFile {
    pub fn new(identity: &Identity) -> Self {
        IdentityFile { master_secret: b64(&identity.master), created_at: crate::config::now_unix() }
    }

    pub fn identity(&self) -> anyhow::Result<Identity> {
        Ok(Identity::from_master(unb64_32(&self.master_secret)?))
    }
}

/// `machine.json`: this Device's secret and what pairing gave it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineFile {
    pub machine_secret: String,
    pub identity_pubkey: String,
    pub content_pubkey: String,
    /// The account DEK, base64url. Encrypts roster, chats, and machine metadata.
    pub account_dek: String,
    pub name: String,
    pub os: String,
    pub os_version: String,
    pub model: String,
    /// Whether the relay has this machine attested by the identity.
    #[serde(default)]
    pub registered: bool,
    /// The identity's relay URL at pairing time. `TINYBOT_RELAY_URL` still overrides.
    #[serde(default)]
    pub relay_url: Option<String>,
    pub created_at: i64,
}

impl MachineFile {
    pub fn machine(&self) -> anyhow::Result<Machine> {
        Ok(Machine::from_secret(unb64_32(&self.machine_secret)?))
    }

    pub fn dek(&self) -> anyhow::Result<[u8; 32]> {
        unb64_32(&self.account_dek)
    }
}

pub fn generate_dek() -> [u8; 32] {
    random_32()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_round_trips() {
        let identity = Identity::generate();
        let phrase = identity.phrase().join(" ");
        assert_eq!(secret_from_phrase(&phrase).unwrap(), identity.master);
        let again = Identity::from_master(secret_from_phrase(&phrase.to_uppercase()).unwrap());
        assert_eq!(again.pubkey(), identity.pubkey());
    }
}
