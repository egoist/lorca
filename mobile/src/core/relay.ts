// HTTP client for the relay (crates/relay). Authenticates this machine with the signature
// challenge, moves ciphertext, and speaks the pairing mailbox.

import { b64, nowUnix, unb64, utf8 } from "./bytes";
import type { Machine } from "./keys";

export class RelayError extends Error {
  status: number | null;
  constructor(message: string, status: number | null = null) {
    super(message);
    this.status = status;
  }
  get isUnauthorized(): boolean {
    return this.status === 401;
  }
  get isClientError(): boolean {
    return this.status !== null && this.status >= 400 && this.status < 500;
  }
}

export interface BlobIn {
  id: string;
  kind: string;
  recipient_machine_pubkey: string | null;
  seq: number;
  ciphertext: string;
  created_at: number;
}

export interface MachineIn {
  machine_pubkey: string;
  box_pubkey: string;
  last_seen: number;
  created_at: number;
}

interface RequestOptions {
  method?: string;
  token?: string;
  body?: unknown;
  timeoutMs?: number;
  signal?: AbortSignal;
}

export class RelayClient {
  private token: { value: string; expiresAt: number } | null = null;

  forgetToken() {
    this.token = null;
  }

  private async request(url: string, options: RequestOptions = {}): Promise<any> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), options.timeoutMs ?? 20_000);
    const onOuterAbort = () => controller.abort();
    options.signal?.addEventListener("abort", onOuterAbort);
    try {
      const headers: Record<string, string> = { Accept: "application/json" };
      if (options.token) headers.Authorization = `Bearer ${options.token}`;
      if (options.body !== undefined) headers["Content-Type"] = "application/json";
      let response: Response;
      try {
        response = await fetch(url, {
          method: options.method ?? "GET",
          headers,
          body: options.body === undefined ? undefined : JSON.stringify(options.body),
          signal: controller.signal,
        });
      } catch (error) {
        if (options.signal?.aborted) throw new RelayError("cancelled");
        throw new RelayError(`relay unreachable: ${error instanceof Error ? error.message : String(error)}`);
      }
      if (response.status === 204) return null;
      const text = await response.text();
      let value: any = null;
      try {
        value = JSON.parse(text);
      } catch {
        value = null;
      }
      if (!response.ok) {
        const message = value?.error ?? `${response.status}: ${text}`;
        throw new RelayError(message, response.status);
      }
      return value;
    } finally {
      clearTimeout(timer);
      options.signal?.removeEventListener("abort", onOuterAbort);
    }
  }

  async health(url: string): Promise<void> {
    await this.request(`${url}/v1/health`, { timeoutMs: 10_000 });
  }

  async authenticate(url: string, machine: Machine): Promise<string> {
    const challenge = await this.request(`${url}/v1/auth/challenge`, {
      method: "POST",
      body: { machine_pubkey: machine.pubkey },
    });
    const nonce: string | undefined = challenge?.nonce;
    if (!nonce) throw new RelayError("no nonce");
    const verified = await this.request(`${url}/v1/auth/verify`, {
      method: "POST",
      body: { machine_pubkey: machine.pubkey, nonce, signature: machine.sign(utf8(nonce)) },
    });
    const token: string | undefined = verified?.token;
    if (!token) throw new RelayError("no token");
    this.token = { value: token, expiresAt: Number(verified.expires_at ?? nowUnix() + 600) };
    return token;
  }

  async tokenFor(url: string, machine: Machine): Promise<string> {
    if (this.token && this.token.expiresAt - 60 > nowUnix()) return this.token.value;
    return this.authenticate(url, machine);
  }

  async putBlob(url: string, token: string, id: string, kind: string, recipient: string | null, ciphertextB64: string): Promise<number> {
    const value = await this.request(`${url}/v1/blobs`, {
      method: "PUT",
      token,
      body: { id, kind, recipient_machine_pubkey: recipient, ciphertext: ciphertextB64 },
    });
    return Number(value?.seq ?? 0);
  }

  async listBlobs(url: string, token: string, since: number, kinds: string, wait: number, signal?: AbortSignal): Promise<{ blobs: BlobIn[]; seq: number }> {
    const query = `since=${since}&kinds=${encodeURIComponent(kinds)}&wait=${wait}`;
    const value = await this.request(`${url}/v1/blobs?${query}`, { token, timeoutMs: (wait + 30) * 1000, signal });
    return { blobs: (value?.blobs ?? []) as BlobIn[], seq: Number(value?.seq ?? since) };
  }

  /// One blob by id, the way `file` blobs are fetched; null when the relay has none.
  async getBlob(url: string, token: string, id: string): Promise<BlobIn | null> {
    try {
      return (await this.request(`${url}/v1/blobs/${encodeURIComponent(id)}`, { token, timeoutMs: 120_000 })) as BlobIn;
    } catch (error) {
      if (error instanceof RelayError && error.status === 404) return null;
      throw error;
    }
  }

  async deleteBlob(url: string, token: string, id: string): Promise<void> {
    await this.request(`${url}/v1/blobs/${encodeURIComponent(id)}`, { method: "DELETE", token });
  }

  async machines(url: string, token: string): Promise<{ machines: MachineIn[]; now: number }> {
    const value = await this.request(`${url}/v1/machines`, { token });
    return { machines: (value?.machines ?? []) as MachineIn[], now: Number(value?.now ?? nowUnix()) };
  }

  // MARK: - Pairing mailbox (the joining side)

  async pairPostRequest(url: string, nonce: string, ciphertext: Uint8Array): Promise<void> {
    await this.request(`${url}/v1/pair/${encodeURIComponent(nonce)}/request`, {
      method: "POST",
      body: { ciphertext: b64(ciphertext) },
    });
  }

  async pairGetReply(url: string, nonce: string): Promise<Uint8Array | null> {
    const value = await this.request(`${url}/v1/pair/${encodeURIComponent(nonce)}/reply`);
    const text: string | null | undefined = value?.ciphertext;
    return text ? unb64(text) : null;
  }
}
