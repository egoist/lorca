// The pairing string the identity Mac shows (as a QR code or text):
// `lorca://pair?relay=…&id=<identity pubkey>&ek=<ephemeral pubkey>&n=<nonce>`. The core does
// the pairing itself; this only checks a scanned or pasted string before it is handed over.

import { t } from "../i18n";

export interface PairingTarget {
  relay: string;
  id: string;
  ek: string;
  nonce: string;
}

export function parsePairingString(text: string): PairingTarget {
  const trimmed = text.trim();
  const index = trimmed.indexOf("pair?");
  if (!trimmed.startsWith("lorca://pair?") && index < 0) {
    throw new Error(t("That is not a Lorca pairing string"));
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
  if (!relay || !id || !ek || !n) throw new Error(t("Pairing string is missing a field"));
  return { relay: relay.replace(/\/+$/, ""), id, ek, nonce: n };
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}
