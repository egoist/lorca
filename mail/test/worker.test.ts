// The Worker's email() handler against a stand-in relay, with messages built the way Email
// Routing hands them over: the envelope addresses, the raw MIME as a stream, and its size.

import { afterAll, beforeAll, expect, test } from "bun:test";
import { x25519 } from "@noble/curves/ed25519.js";

import worker from "../src/index";
import { addressName, MAX_MAIL_BYTES, type Env, type IncomingMail } from "../src/mail";
import { base64url, fromBase64url, open, seal } from "../src/seal";
import vector from "./seal-vector.json";

const TOKEN = "worker-token";
const runners = [x25519.utils.randomSecretKey(), x25519.utils.randomSecretKey()];

interface Delivery {
  path: string;
  authorization: string | null;
  body: { name: string; machine_pubkey: string; ciphertext: string };
}

// What the stand-in relay answers, by address name.
let routes: Record<string, { status: number; body?: unknown }> = {};
let deliveryStatus = 200;
let deliveries: Delivery[] = [];
let server: ReturnType<typeof Bun.serve>;
let env: Env;

beforeAll(() => {
  server = Bun.serve({
    port: 0,
    async fetch(request) {
      const url = new URL(request.url);
      if (request.headers.get("authorization") !== `Bearer ${TOKEN}`) return Response.json({ error: "Not the mail Worker" }, { status: 401 });
      const route = url.pathname.match(/^\/v1\/mail\/route\/(.+)$/);
      if (route && request.method === "GET") {
        const answer = routes[decodeURIComponent(route[1]!)] ?? { status: 404, body: { error: "No such address" } };
        return Response.json(answer.body ?? {}, { status: answer.status });
      }
      if (url.pathname.startsWith("/v1/mail/deliveries/") && request.method === "PUT") {
        deliveries.push({ path: url.pathname, authorization: request.headers.get("authorization"), body: await request.json() });
        return Response.json({ seq: deliveries.length }, { status: deliveryStatus });
      }
      return new Response("not found", { status: 404 });
    },
  });
  env = { RELAY_URL: `http://127.0.0.1:${server.port}/`, MAIL_TOKEN: TOKEN };
});

afterAll(() => server.stop(true));

function machines() {
  return runners.map((secret, index) => ({ machine_pubkey: `machine-${index}`, box_pubkey: base64url(x25519.getPublicKey(secret)) }));
}

const RAW = [
  "From: Acme Support <help@acme.example>",
  "To: k7f3m9q2+scout@bots.lorca.app",
  "Subject: Your code",
  "Message-ID: <code-1@acme.example>",
  "Content-Type: text/plain; charset=utf-8",
  "",
  "Your verification code is 482913.",
  "",
].join("\r\n");

function mail(to: string, raw: string | Uint8Array = RAW, rawSize?: number) {
  const bytes = typeof raw === "string" ? new TextEncoder().encode(raw) : raw;
  let rejected: string | null = null;
  let read = false;
  const message: IncomingMail = {
    from: "bounce@acme.example",
    to,
    rawSize: rawSize ?? bytes.length,
    get raw() {
      read = true;
      return new Response(bytes).body!;
    },
    setReject(reason: string) {
      rejected = reason;
    },
  };
  return { message, rejected: () => rejected, read: () => read };
}

function reset(route?: { status: number; body?: unknown }) {
  routes = route ? { k7f3m9q2: route } : {};
  deliveries = [];
  deliveryStatus = 200;
}

async function handle(message: IncomingMail) {
  await worker.email(message as unknown as ForwardableEmailMessage, env);
}

test("each Runner gets a copy only its box key opens", async () => {
  reset({ status: 200, body: { state: "active", machines: machines() } });
  const { message, rejected } = mail("K7F3M9Q2+Scout@bots.lorca.app");
  await handle(message);
  expect(rejected()).toBeNull();
  expect(deliveries).toHaveLength(2);
  const id = deliveries[0]!.path.split("/").pop()!.split(".")[0]!;
  expect(deliveries.map((d) => d.path).sort()).toEqual([`/v1/mail/deliveries/${id}.0`, `/v1/mail/deliveries/${id}.1`]);
  for (const [index, secret] of runners.entries()) {
    const delivery = deliveries.find((d) => d.body.machine_pubkey === `machine-${index}`)!;
    expect(delivery.body.name).toBe("k7f3m9q2");
    const plaintext = new TextDecoder().decode(open(fromBase64url(delivery.body.ciphertext), secret));
    const [magic, header, ...rest] = plaintext.split("\n");
    expect(magic).toBe("lorca-mail/1");
    expect(JSON.parse(header!)).toMatchObject({ id, from: "bounce@acme.example", to: "K7F3M9Q2+Scout@bots.lorca.app" });
    expect(rest.join("\n")).toBe(RAW);
    // The other Runner's key opens nothing.
    expect(() => open(fromBase64url(delivery.body.ciphertext), runners[1 - index]!)).toThrow();
  }
});

test("mail to an address nobody holds, a suspended one, or too large a message is refused", async () => {
  reset();
  let message = mail("nobody@bots.lorca.app");
  await handle(message.message);
  expect(message.rejected()).toBe("No such address");

  reset({ status: 200, body: { state: "suspended", machines: [] } });
  message = mail("k7f3m9q2@bots.lorca.app");
  await handle(message.message);
  expect(message.rejected()).toBe("Address suspended");

  reset({ status: 200, body: { state: "active", machines: machines() } });
  message = mail("k7f3m9q2@bots.lorca.app", RAW, MAX_MAIL_BYTES + 1);
  await handle(message.message);
  expect(message.rejected()).toBe("Message too large");
  expect(message.read()).toBe(false);

  message = mail("not an address");
  await handle(message.message);
  expect(message.rejected()).toBe("No such address");
  expect(deliveries).toHaveLength(0);
});

test("what may pass leaves the message with the sender's server", async () => {
  reset({ status: 200, body: { state: "active", machines: [] } });
  expect(handle(mail("k7f3m9q2@bots.lorca.app").message)).rejects.toThrow("No Runner");

  reset({ status: 503, body: { error: "unavailable" } });
  expect(handle(mail("k7f3m9q2@bots.lorca.app").message)).rejects.toThrow("503");

  reset({ status: 200, body: { state: "active", machines: machines() } });
  deliveryStatus = 500;
  expect(handle(mail("k7f3m9q2@bots.lorca.app").message)).rejects.toThrow("500");
});

test("an account out of room bounces the message", async () => {
  reset({ status: 200, body: { state: "active", machines: machines() } });
  deliveryStatus = 413;
  const { message, rejected } = mail("k7f3m9q2@bots.lorca.app");
  await handle(message);
  expect(rejected()).toBe("Mailbox full");
});

test("the address's name drops a bot's tag", () => {
  expect(addressName("K7F3M9Q2+Scout@bots.lorca.app")).toBe("k7f3m9q2");
  expect(addressName("<egoist.dev@bots.lorca.app>")).toBe("egoist.dev");
  expect(addressName("ab@bots.lorca.app")).toBeNull();
  expect(addressName("../x@bots.lorca.app")).toBeNull();
});

test("the seal matches the vector a Runner's test opens", () => {
  const plaintext = fromBase64url(vector.plaintext);
  const sealed = seal(plaintext, fromBase64url(vector.box_public), fromBase64url(vector.ephemeral_secret));
  expect(base64url(sealed)).toBe(vector.sealed);
  expect(open(sealed, fromBase64url(vector.box_secret))).toEqual(plaintext);
});
