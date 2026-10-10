// libsodium's sealed box (`crypto_box_seal`), which the Rust side opens with the `crypto_box`
// crate: an ephemeral X25519 key, a nonce of BLAKE2b(ephemeral public key ‖ recipient's public
// key), and XSalsa20-Poly1305 under the HSalsa20 of the shared secret. A copy sealed this way
// opens only with the recipient Runner's box secret.

import { hsalsa, xsalsa20poly1305 } from "@noble/ciphers/salsa.js";
import { u32 } from "@noble/ciphers/utils.js";
import { x25519 } from "@noble/curves/ed25519.js";
import { blake2b } from "@noble/hashes/blake2.js";

// "expand 32-byte k"
const SIGMA = new Uint32Array([0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

// `crypto_box_beforenm`: the XSalsa20-Poly1305 key two parties share.
function boxKey(secret: Uint8Array, publicKey: Uint8Array): Uint8Array {
  const shared = x25519.getSharedSecret(secret, publicKey);
  const out = new Uint32Array(8);
  hsalsa(SIGMA, u32(Uint8Array.from(shared)), new Uint32Array(4), out);
  return new Uint8Array(out.buffer);
}

function nonce(ephemeralPublic: Uint8Array, recipient: Uint8Array): Uint8Array {
  return blake2b(concat(ephemeralPublic, recipient), { dkLen: 24 });
}

// `ephemeralSecret` is for test vectors; a real seal takes a fresh one.
export function seal(message: Uint8Array, recipient: Uint8Array, ephemeralSecret: Uint8Array = x25519.utils.randomSecretKey()): Uint8Array {
  if (recipient.length !== 32) throw new Error("A box public key is 32 bytes");
  const ephemeralPublic = x25519.getPublicKey(ephemeralSecret);
  const boxed = xsalsa20poly1305(boxKey(ephemeralSecret, recipient), nonce(ephemeralPublic, recipient)).encrypt(message);
  return concat(ephemeralPublic, boxed);
}

export function open(sealed: Uint8Array, secret: Uint8Array): Uint8Array {
  const ephemeralPublic = sealed.slice(0, 32);
  const recipient = x25519.getPublicKey(secret);
  return xsalsa20poly1305(boxKey(secret, ephemeralPublic), nonce(ephemeralPublic, recipient)).decrypt(sealed.slice(32));
}

export function base64url(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function fromBase64url(text: string): Uint8Array {
  const base64 = text.replaceAll("-", "+").replaceAll("_", "/");
  return Uint8Array.from(atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4)), (c) => c.charCodeAt(0));
}

export { concat };
