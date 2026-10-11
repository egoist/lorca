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
    let hkdf = Hkdf::<Sha256>::new(Some(b"lorca-v1"), secret);
    let mut out = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut out).expect("hkdf expand");
    out
}

/// The key pushes are sealed under, so the phone's notification extension holds this one
/// key and never the account DEK.
pub fn push_key(dek: &[u8; 32]) -> [u8; 32] {
    derive(dek, "push")
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
        .map_err(|_| anyhow::anyhow!("That does not look like a Lorca backup phrase"))?;
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
    /// Whether the relay has this machine attested: by the identity, or by the Device that
    /// paired it.
    #[serde(default)]
    pub registered: bool,
    /// The identity's relay URL at pairing time. `LORCA_RELAY_URL` still overrides.
    #[serde(default)]
    pub relay_url: Option<String>,
    pub created_at: i64,
    /// The account key's generations as the identity signed them and they reached this Device,
    /// each checked when it came. `account_dek` is generation 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyrings: Vec<SignedKeyring>,
}

impl MachineFile {
    pub fn machine(&self) -> anyhow::Result<Machine> {
        Ok(Machine::from_secret(unb64_32(&self.machine_secret)?))
    }

    /// The account keys this Device holds: the newest encrypts, any opens.
    pub fn dek(&self) -> anyhow::Result<AccountKeys> {
        let keyring = self.keyring()?;
        let mut keys = keyring.keys.iter().map(|key| unb64_32(&key.key)).collect::<anyhow::Result<Vec<_>>>()?;
        keys.reverse();
        Ok(AccountKeys(keys))
    }

    /// Every generation of the account key this Device holds, oldest first.
    pub fn keyring(&self) -> anyhow::Result<Keyring> {
        let mut keys = vec![GenerationKey { generation: 0, key: self.account_dek.clone(), made_at: 0, unpaired: Default::default() }];
        for signed in &self.keyrings {
            let keyring = signed.read()?;
            // Only this account's: one that begins with another generation 0 says nothing here.
            if keyring.keys.iter().any(|key| key.generation == 0 && key.key == self.account_dek) {
                keys.extend(keyring.keys);
            }
        }
        keys.sort_by(|a, b| (a.generation, &a.key).cmp(&(b.generation, &b.key)));
        keys.dedup_by(|a, b| a.key == b.key);
        Ok(Keyring { identity: self.identity_pubkey.clone(), keys })
    }

    /// Keeps a keyring another Device sent, once the identity's signature checks and it begins
    /// with this account's generation 0. Answers whether it brought a newer key.
    pub fn add_keyring(&mut self, signed: SignedKeyring) -> anyhow::Result<bool> {
        let incoming = signed.verify(&self.identity_pubkey)?;
        if !incoming.keys.iter().any(|key| key.generation == 0 && key.key == self.account_dek) {
            anyhow::bail!("these account keys belong to another account");
        }
        let held = self.keyring()?;
        if incoming.keys.iter().all(|key| held.keys.iter().any(|kept| kept.key == key.key)) {
            return Ok(false);
        }
        let newest = held.newest().clone();
        // A keyring another one holds whole says nothing more.
        self.keyrings.retain(|kept| kept.read().is_ok_and(|kept| !kept.is_within(&incoming)));
        self.keyrings.push(signed);
        Ok(self.keyring()?.newest() != &newest)
    }
}

/// The account keys a Device holds, newest first. The newest encrypts what this Device writes,
/// to the relay and to its own files; any of them opens what was written before. An account
/// whose key never changed has one: the account DEK it was made with.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountKeys(Vec<[u8; 32]>);

impl AccountKeys {
    pub fn single(key: [u8; 32]) -> Self {
        AccountKeys(vec![key])
    }

    pub fn newest(&self) -> &[u8; 32] {
        &self.0[0]
    }

    pub fn each(&self) -> &[[u8; 32]] {
        &self.0
    }
}

impl std::fmt::Debug for AccountKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AccountKeys({} generations)", self.0.len())
    }
}

/// Signed ahead of a keyring, so the identity's signature on one passes for nothing else.
const KEYRING_DOMAIN: &[u8] = b"lorca/keyring/v1\0";

/// One generation of the account key, base64url. Generation 0 is the account DEK the account was
/// made with; each later one was made when a Device was unpaired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationKey {
    pub generation: u32,
    pub key: String,
    /// When the identity device made it, in Unix seconds; 0 for generation 0.
    #[serde(default)]
    pub made_at: i64,
    /// The Devices unpaired when it was made, by machine pubkey: they never get it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub unpaired: std::collections::BTreeSet<String>,
}

/// The account key's generations, oldest first, by the order of `(generation, key)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyring {
    pub identity: String,
    pub keys: Vec<GenerationKey>,
}

impl Keyring {
    pub fn newest(&self) -> &GenerationKey {
        self.keys.last().expect("a keyring holds generation 0")
    }

    /// Every Device unpaired when any generation was made.
    pub fn unpaired(&self) -> std::collections::BTreeSet<String> {
        self.keys.iter().flat_map(|key| key.unpaired.iter().cloned()).collect()
    }

    fn is_within(&self, other: &Keyring) -> bool {
        self.keys.iter().all(|key| other.keys.iter().any(|kept| kept.key == key.key))
    }
}

/// A keyring and the identity's signature over it. Only an identity device makes one; any
/// Device passes one on, and every Device checks the signature against the identity it belongs
/// to before it keeps one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedKeyring {
    pub keyring: String,
    pub signature: String,
}

impl SignedKeyring {
    pub fn sign(identity: &Identity, keyring: &Keyring) -> anyhow::Result<Self> {
        let bytes = serde_json::to_vec(keyring)?;
        Ok(SignedKeyring { keyring: b64(&bytes), signature: identity.sign(&[KEYRING_DOMAIN, &bytes].concat()) })
    }

    /// The keyring, when the identity `identity_pubkey` signed it as its own.
    pub fn verify(&self, identity_pubkey: &str) -> anyhow::Result<Keyring> {
        let bytes = unb64(&self.keyring)?;
        let signature = ed25519_dalek::Signature::from_slice(&unb64(&self.signature)?)?;
        verifying_key(identity_pubkey)?
            .verify_strict(&[KEYRING_DOMAIN, &bytes].concat(), &signature)
            .map_err(|_| anyhow::anyhow!("the account keys are not signed by this account's identity"))?;
        let keyring: Keyring = serde_json::from_slice(&bytes)?;
        if keyring.identity != identity_pubkey {
            anyhow::bail!("the account keys name another identity");
        }
        Ok(keyring)
    }

    /// The keyring as kept in `machine.json`, checked when it came.
    pub fn read(&self) -> anyhow::Result<Keyring> {
        Ok(serde_json::from_slice(&unb64(&self.keyring)?)?)
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
