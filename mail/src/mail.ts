// What the mail Worker does with a message. Email Routing's catch-all on the mail domain hands
// it every message; it asks the relay where the address's mail goes, reads the message once,
// seals a copy to each Runner of the account, and hands the copies to the relay as `event`
// envelopes that only that Runner opens. It keeps nothing. Mail to an address nobody holds, a suspended one, or one over the
// size a Runner takes is refused with a permanent error; anything that may pass (the relay
// unreachable, no Runner ready yet) throws, which Email Routing answers with a temporary failure
// so the sender's server tries again.

import { base64url, concat, fromBase64url, seal } from "./seal";

export interface Env {
  // The relay that keeps the addresses, `https://relay.lorca.app`.
  RELAY_URL: string;
  // The bearer the relay takes from this Worker (its --mail-token).
  MAIL_TOKEN: string;
}

// An `event` envelope holds 4 MiB of ciphertext; the sealed header and the seal take the rest.
export const MAX_MAIL_BYTES = 4 * 1024 * 1024 - 4096;
// The sealed plaintext: this line, a JSON header line, then the raw message.
const MAGIC = new TextEncoder().encode("lorca-mail/1\n");
// The relay protocol this Worker speaks (`Lorca-Protocol`).
const PROTOCOL = "3";

// The part of the message Email Routing gives the handler that this Worker uses.
export interface IncomingMail {
  readonly from: string;
  readonly to: string;
  readonly raw: ReadableStream<Uint8Array>;
  readonly rawSize: number;
  setReject(reason: string): void;
}

// `k7f3m9q2` from `K7F3M9Q2+Scout@bots.lorca.app`: the address's name, without a bot's tag.
export function addressName(recipient: string): string | null {
  const local = recipient.trim().replace(/^<|>$/g, "").split("@")[0]?.split("+")[0]?.toLowerCase();
  return local && /^[a-z0-9][a-z0-9.-]{1,30}[a-z0-9]$/.test(local) ? local : null;
}

function relay(env: Env, path: string, init: RequestInit = {}): Promise<Response> {
  return fetch(env.RELAY_URL.replace(/\/+$/, "") + path, {
    ...init,
    headers: { authorization: `Bearer ${env.MAIL_TOKEN}`, "lorca-protocol": PROTOCOL, "content-type": "application/json", ...init.headers },
  });
}

interface Route {
  state: "active" | "suspended";
  machines: { machine_pubkey: string; box_pubkey: string }[];
}

export async function receive(message: IncomingMail, env: Env): Promise<void> {
  const name = addressName(message.to);
  if (!name) {
    message.setReject("No such address");
    return;
  }
  if (message.rawSize > MAX_MAIL_BYTES) {
    message.setReject("Message too large");
    return;
  }
  const answer = await relay(env, `/v1/mail/route/${encodeURIComponent(name)}`);
  if (answer.status === 404) {
    message.setReject("No such address");
    return;
  }
  if (!answer.ok) throw new Error(`The relay answered ${answer.status}`);
  const route = (await answer.json()) as Route;
  if (route.state === "suspended") {
    message.setReject("Address suspended");
    return;
  }
  // Mail waits with the sender until a Runner of the account takes it.
  if (route.machines.length === 0) throw new Error("No Runner takes this address's mail yet");

  const raw = new Uint8Array(await new Response(message.raw).arrayBuffer());
  if (raw.length > MAX_MAIL_BYTES) {
    message.setReject("Message too large");
    return;
  }
  const id = crypto.randomUUID().replaceAll("-", "");
  const header = JSON.stringify({ id, received_at: Math.floor(Date.now() / 1000), from: message.from, to: message.to });
  const plaintext = concat(MAGIC, new TextEncoder().encode(header + "\n"), raw);
  const refused = await Promise.all(
    route.machines.map(async (machine, index) => {
      const sealed = seal(plaintext, fromBase64url(machine.box_pubkey));
      const answer = await relay(env, `/v1/mail/deliveries/${id}.${index}`, {
        method: "PUT",
        body: JSON.stringify({ name, machine_pubkey: machine.machine_pubkey, ciphertext: base64url(sealed) }),
      });
      if (answer.status === 413) return true;
      if (!answer.ok) throw new Error(`The relay answered ${answer.status}`);
      return false;
    }),
  );
  // The account has no room left. A copy that went in stays; the Runners keep it once.
  if (refused.some(Boolean)) message.setReject("Mailbox full");
}
