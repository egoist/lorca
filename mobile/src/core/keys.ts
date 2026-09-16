// This Device's keys. A 32-byte machine secret → HKDF-SHA256 (salt "tinybot-v1") → an Ed25519
// signing key ("machine/sign") and an X25519 box key ("machine/box"). Public keys travel as
// base64url; the identity id is the first eight bytes of sha256(identity pubkey), hex.
// Mirrors crates/cli/src/keys.rs.

import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { b64, hex, randomBytes, unb64_32, utf8 } from "./bytes";

function derive(secret: Uint8Array, info: string): Uint8Array {
  return hkdf(sha256, secret, utf8("tinybot-v1"), utf8(info), 32);
}

export class Machine {
  readonly secret: Uint8Array;
  readonly signingSecret: Uint8Array;
  readonly boxSecret: Uint8Array;
  readonly pubkey: string;
  readonly boxPubkey: string;

  constructor(secret: Uint8Array) {
    this.secret = secret;
    this.signingSecret = derive(secret, "machine/sign");
    this.boxSecret = derive(secret, "machine/box");
    this.pubkey = b64(ed25519.getPublicKey(this.signingSecret));
    this.boxPubkey = b64(x25519.getPublicKey(this.boxSecret));
  }

  static generate(): Machine {
    return new Machine(randomBytes(32));
  }

  static fromSecretB64(text: string): Machine {
    return new Machine(unb64_32(text));
  }

  sign(message: Uint8Array): string {
    return b64(ed25519.sign(message, this.signingSecret));
  }
}

export function identityId(pubkey: string): string {
  return hex(sha256(utf8(pubkey)).subarray(0, 8));
}

/// What pairing gave this Device: its secret and the account it belongs to. Kept in the
/// secure store, the phone's `machine.json`.
export interface MachineFile {
  machine_secret: string;
  identity_pubkey: string;
  content_pubkey: string;
  /// The account DEK, base64url. Encrypts roster, chats, and machine metadata.
  account_dek: string;
  name: string;
  os: string;
  os_version: string;
  model: string;
  relay_url: string;
  created_at: number;
}
