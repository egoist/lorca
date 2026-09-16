// Joining an identity from a pairing string the identity Mac shows (as a QR code or text):
// `tinybot://pair?relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`. This Device
// generates its machine keys, posts a request sealed to `ek` into the relay's pairing mailbox,
// and polls for the reply sealed to its own box key, which carries the account key.

import { b64, nowUnix } from "./bytes";
import { sealJson, unsealJson } from "./crypto";
import { Machine, type MachineFile } from "./keys";
import type { Device as DeviceModel, PairReply, PairRequest } from "./model";
import type { RelayClient } from "./relay";

const PAIR_TIMEOUT_MS = 10 * 60 * 1000;
const POLL_MS = 1500;

export interface PairingTarget {
  relay: string;
  id: string;
  ek: string;
  nonce: string;
}

export function parsePairingString(text: string): PairingTarget {
  const trimmed = text.trim();
  const index = trimmed.indexOf("pair?");
  if (!trimmed.startsWith("tinybot://pair?") && index < 0) {
    throw new Error("That is not a Tinybot pairing string");
  }
  const query = trimmed.slice(trimmed.indexOf("pair?") + "pair?".length);
  const fields: Record<string, string> = {};
  for (const pair of query.split("&")) {
    const eq = pair.indexOf("=");
    const key = eq < 0 ? pair : pair.slice(0, eq);
    const value = eq < 0 ? "" : pair.slice(eq + 1);
    fields[key] = safeDecode(value);
  }
  const { relay, id, ek, n } = fields;
  if (!relay || !id || !ek || !n) throw new Error("Pairing string is missing a field");
  return { relay: relay.replace(/\/+$/, ""), id, ek, nonce: n };
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export interface PairProgress {
  phase: "posting" | "waiting" | "done";
}

export interface HostFacts {
  os: string;
  os_version: string;
  model: string;
  name: string;
}

/// Joins the identity behind `pairingString`. Resolves with the machine file to keep.
export async function acceptPairing(
  relay: RelayClient,
  pairingString: string,
  host: HostFacts,
  onProgress?: (progress: PairProgress) => void,
  signal?: AbortSignal,
): Promise<{ file: MachineFile; device: DeviceModel; machine: Machine }> {
  const target = parsePairingString(pairingString);
  await relay.health(target.relay);

  const machine = Machine.generate();
  const device: DeviceModel = {
    id: machine.pubkey,
    name: host.name,
    model: host.model,
    os: host.os,
    os_version: host.os_version,
    box_pubkey: machine.boxPubkey,
    providers_connected: [],
    updated_at: nowUnix(),
  };
  const request: PairRequest = { machine_pubkey: machine.pubkey, box_pubkey: machine.boxPubkey, device };
  onProgress?.({ phase: "posting" });
  await relay.pairPostRequest(target.relay, target.nonce, sealJson(target.ek, request));
  onProgress?.({ phase: "waiting" });

  const deadline = Date.now() + PAIR_TIMEOUT_MS;
  let reply: PairReply | null = null;
  while (!reply) {
    if (signal?.aborted) throw new Error("Pairing cancelled");
    if (Date.now() > deadline) throw new Error("The other Device did not answer in time");
    const sealed = await relay.pairGetReply(target.relay, target.nonce);
    if (sealed) {
      reply = unsealJson<PairReply>(machine.boxSecret, sealed);
    } else {
      await new Promise((resolve) => setTimeout(resolve, POLL_MS));
    }
  }
  if (reply.identity_pubkey !== target.id) {
    throw new Error("The reply came from a different identity than the pairing string");
  }
  onProgress?.({ phase: "done" });
  const file: MachineFile = {
    machine_secret: b64(machine.secret),
    identity_pubkey: reply.identity_pubkey,
    content_pubkey: reply.content_pubkey,
    account_dek: reply.account_dek,
    name: device.name,
    os: device.os,
    os_version: device.os_version,
    model: device.model,
    relay_url: reply.relay_url.replace(/\/+$/, ""),
    created_at: nowUnix(),
  };
  return { file, device, machine };
}
