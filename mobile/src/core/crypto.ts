// The CLI's crypto, in TypeScript: XChaCha20-Poly1305 envelopes keyed by the account DEK with
// the blob kind as associated data, and libsodium sealed boxes for anything addressed to one
// X25519 public key (jobs, pairing, wrapped DEKs). Byte-for-byte compatible with
// crates/cli/src/crypto.rs.

import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { xsalsa20poly1305 } from "@noble/ciphers/salsa.js";
import { hsalsa } from "@noble/ciphers/salsa.js";
import { x25519 } from "@noble/curves/ed25519.js";
import { blake2b } from "@noble/hashes/blake2.js";
import { concat, fromUtf8, randomBytes, unb64_32, utf8 } from "./bytes";

const NONCE_LEN = 24;

/// `nonce || ciphertext`, with the blob kind as associated data.
export function encrypt(dek: Uint8Array, kind: string, plaintext: Uint8Array): Uint8Array {
  const nonce = randomBytes(NONCE_LEN);
  const ciphertext = xchacha20poly1305(dek, nonce, utf8(kind)).encrypt(plaintext);
  return concat(nonce, ciphertext);
}

export function decrypt(dek: Uint8Array, kind: string, envelope: Uint8Array): Uint8Array {
  if (envelope.length <= NONCE_LEN) throw new Error("envelope too short");
  const nonce = envelope.subarray(0, NONCE_LEN);
  const ciphertext = envelope.subarray(NONCE_LEN);
  try {
    return xchacha20poly1305(dek, nonce, utf8(kind)).decrypt(ciphertext);
  } catch {
    throw new Error("decryption failed");
  }
}

export function encryptJson(dek: Uint8Array, kind: string, value: unknown): Uint8Array {
  return encrypt(dek, kind, utf8(JSON.stringify(value)));
}

export function decryptJson<T>(dek: Uint8Array, kind: string, envelope: Uint8Array): T {
  return JSON.parse(fromUtf8(decrypt(dek, kind, envelope))) as T;
}

// MARK: - Sealed boxes (crypto_box_seal)
//
// ephemeral_pk || crypto_box(m, nonce = blake2b-24(ephemeral_pk || recipient_pk), recipient_pk, ephemeral_sk)
// crypto_box = xsalsa20poly1305 keyed with hsalsa20(x25519(sk, pk)).

const SIGMA = new Uint32Array(new Uint8Array(utf8("expand 32-byte k")).buffer);
const ZERO16 = new Uint32Array(4);

function boxKey(secret: Uint8Array, publicKey: Uint8Array): Uint8Array {
  const shared = x25519.getSharedSecret(secret, publicKey);
  const out = new Uint32Array(8);
  hsalsa(SIGMA, new Uint32Array(shared.buffer, shared.byteOffset, 8), ZERO16, out);
  return new Uint8Array(out.buffer);
}

function sealNonce(ephemeralPublic: Uint8Array, recipientPublic: Uint8Array): Uint8Array {
  return blake2b(concat(ephemeralPublic, recipientPublic), { dkLen: 24 });
}

/// Seal to a base64url X25519 public key.
export function seal(recipientBoxPubkey: string, plaintext: Uint8Array): Uint8Array {
  const recipient = unb64_32(recipientBoxPubkey);
  const ephemeralSecret = randomBytes(32);
  const ephemeralPublic = x25519.getPublicKey(ephemeralSecret);
  const key = boxKey(ephemeralSecret, recipient);
  const nonce = sealNonce(ephemeralPublic, recipient);
  const ciphertext = xsalsa20poly1305(key, nonce).encrypt(plaintext);
  return concat(ephemeralPublic, ciphertext);
}

export function sealJson(recipientBoxPubkey: string, value: unknown): Uint8Array {
  return seal(recipientBoxPubkey, utf8(JSON.stringify(value)));
}

export function unseal(boxSecret: Uint8Array, ciphertext: Uint8Array): Uint8Array {
  if (ciphertext.length < 32 + 16) throw new Error("unseal failed: too short");
  const ephemeralPublic = ciphertext.subarray(0, 32);
  const recipientPublic = x25519.getPublicKey(boxSecret);
  const key = boxKey(boxSecret, ephemeralPublic);
  const nonce = sealNonce(ephemeralPublic, recipientPublic);
  try {
    return xsalsa20poly1305(key, nonce).decrypt(ciphertext.subarray(32));
  } catch {
    throw new Error("unseal failed: not addressed to this key");
  }
}

export function unsealJson<T>(boxSecret: Uint8Array, ciphertext: Uint8Array): T {
  return JSON.parse(fromUtf8(unseal(boxSecret, ciphertext))) as T;
}
