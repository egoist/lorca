// Lorca's Cloudflare Sandbox: a Worker the user deploys to their own Cloudflare account, so a
// Runner's coding agents run in Containers there. Each sandbox is a Durable Object that starts
// its Container, runs commands in it, and passes a process's stdin, stdout, and stderr over a
// WebSocket. Every request needs the LORCA_TOKEN secret as a bearer token.
//
// The API, version 1, as the Runner speaks it (crates/cli/src/coding/sandbox/cloudflare.rs):
//   GET    /health                    { lorca: 1 }
//   POST   /sandboxes/{name}/start    starts the Container unless it runs
//   POST   /sandboxes/{name}/exec     { argv, cwd, env } -> { exitCode, stdout, stderr }
//   GET    /sandboxes/{name}/spawn    WebSocket: first message { argv, cwd, env }; then binary
//                                     messages are stdin one way and output the other (first
//                                     byte 1 for stdout, 2 for stderr); { signal: 15 } stops the
//                                     process; { exit: code } says it ended
//   DELETE /sandboxes/{name}          ends the Container and its files

import { DurableObject } from "cloudflare:workers";

interface Env {
  SANDBOX: DurableObjectNamespace<LorcaSandbox>;
  LORCA_TOKEN: string;
}

const PROTOCOL = 1;
const NAME = /^[a-z0-9][a-z0-9-]{0,62}$/;
// Longer than the hour after which the Runner stops an agent that is done and quiet, so a
// follow-up within it finds the clone still there.
const INACTIVITY_TIMEOUT_MS = 2 * 60 * 60 * 1_000;
// Requests keep a Container awake and a running process does not, so while a process is
// attached the Durable Object asks the Container something this often.
const KEEP_AWAKE_MS = 60 * 1_000;

interface Command {
  argv: string[];
  cwd?: string;
  env?: Record<string, string>;
}

export class LorcaSandbox extends DurableObject<Env> {
  readonly #container: Container;
  #attached = 0;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    const container = ctx.container;
    if (container === undefined) throw new Error("The container binding is not configured");
    this.#container = container;
    // Each Durable Object instance sets its own timeout; it is not inherited.
    if (container.running) {
      void ctx.blockConcurrencyWhile(() => container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS));
    }
  }

  async start(name: string): Promise<void> {
    if (!this.#container.running) {
      this.#container.start({
        image: this.#container.images.sandbox,
        instance: "standard-1",
        // Agents push, open pull requests, and call their model providers.
        enableInternet: true,
        labels: { lorca: name },
      });
    }
    await this.#container.setInactivityTimeout(INACTIVITY_TIMEOUT_MS);
  }

  async exec(name: string, command: Command): Promise<{ exitCode: number; stdout: string; stderr: string }> {
    await this.start(name);
    const process = await this.#container.exec(command.argv, { cwd: command.cwd, env: command.env, stdout: "pipe", stderr: "pipe" });
    const output = await process.output();
    const decoder = new TextDecoder();
    return { exitCode: output.exitCode, stdout: decoder.decode(output.stdout), stderr: decoder.decode(output.stderr) };
  }

  async destroy(): Promise<void> {
    await this.ctx.storage.deleteAlarm();
    if (this.#container.running) await this.#container.destroy();
  }

  async alarm(): Promise<void> {
    if (this.#attached === 0 || !this.#container.running) return;
    const probe = await this.#container.exec(["true"], { stdout: "ignore", stderr: "ignore" });
    await probe.exitCode;
    await this.ctx.storage.setAlarm(Date.now() + KEEP_AWAKE_MS);
  }

  // A process the Runner follows: its streams go over this WebSocket for as long as it runs.
  override async fetch(request: Request): Promise<Response> {
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("Expected a WebSocket upgrade", { status: 426 });
    }
    const name = new URL(request.url).searchParams.get("sandbox") ?? "";
    await this.start(name);
    const [client, server] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket];
    // An accepted socket, not a hibernating one, keeps this instance and the process handle.
    server.accept();
    this.#bridge(server);
    return new Response(null, { status: 101, webSocket: client });
  }

  #bridge(socket: WebSocket): void {
    let process: ExecProcess | undefined;
    let input: WritableStreamDefaultWriter<Uint8Array> | undefined;
    let exited = false;
    // Messages are handled in the order they came, the first one starting the process.
    let queue: Promise<void> = Promise.resolve();
    const fail = (error: unknown) => {
      try {
        socket.send(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
        socket.close(1011, "The sandbox failed");
      } catch {}
      if (process !== undefined && !exited) process.kill(9);
    };
    const begin = async (command: Command) => {
      process = await this.#container.exec(command.argv, { cwd: command.cwd, env: command.env, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
      input = process.stdin?.getWriter();
      this.#attached++;
      await this.ctx.storage.setAlarm(Date.now() + KEEP_AWAKE_MS);
      const forward = async (stream: ReadableStream<Uint8Array> | null, tag: number) => {
        if (stream === null) return;
        for await (const chunk of stream) {
          const frame = new Uint8Array(chunk.byteLength + 1);
          frame[0] = tag;
          frame.set(chunk, 1);
          socket.send(frame);
        }
      };
      const running = process;
      void (async () => {
        try {
          await Promise.all([forward(running.stdout, 1), forward(running.stderr, 2)]);
          const code = await running.exitCode;
          exited = true;
          socket.send(JSON.stringify({ exit: code }));
          socket.close(1000, "Exited");
        } catch (error) {
          exited = true;
          fail(error);
        } finally {
          this.#attached--;
        }
      })();
    };
    socket.addEventListener("message", (event) => {
      queue = queue
        .then(async () => {
          if (process === undefined) {
            await begin(JSON.parse(String(event.data)) as Command);
            return;
          }
          if (typeof event.data === "string") {
            const message = JSON.parse(event.data) as { signal?: number };
            if (message.signal !== undefined && !exited) process.kill(message.signal);
            return;
          }
          await input?.write(new Uint8Array(event.data as ArrayBuffer));
        })
        .catch(fail);
    });
    // The Runner went away: what it followed ends with it.
    const gone = () => {
      if (process !== undefined && !exited) process.kill(15);
    };
    socket.addEventListener("close", gone);
    socket.addEventListener("error", gone);
  }
}

async function authorized(request: Request, env: Env): Promise<boolean> {
  const expected = env.LORCA_TOKEN ?? "";
  const given = (request.headers.get("Authorization") ?? "").replace(/^Bearer /, "");
  if (expected === "" || given.length !== expected.length) return false;
  const encoder = new TextEncoder();
  return crypto.subtle.timingSafeEqual(encoder.encode(given), encoder.encode(expected));
}

export default {
  async fetch(request, env): Promise<Response> {
    if (!(await authorized(request, env))) return new Response("Unauthorized", { status: 401 });
    const url = new URL(request.url);
    if (url.pathname === "/health" && request.method === "GET") return Response.json({ lorca: PROTOCOL });
    const match = /^\/sandboxes\/([^/]+)(?:\/(start|exec|spawn))?$/.exec(url.pathname);
    if (match === null) return new Response("Not found", { status: 404 });
    const [, name = "", action] = match;
    if (!NAME.test(name)) return new Response("A sandbox's name is 1-63 lowercase letters, digits, or hyphens", { status: 400 });
    const sandbox = env.SANDBOX.getByName(name);
    try {
      if (action === "start" && request.method === "POST") {
        await sandbox.start(name);
        return new Response(null, { status: 204 });
      }
      if (action === "exec" && request.method === "POST") {
        const command = (await request.json()) as Command;
        if (!Array.isArray(command.argv) || command.argv.length === 0) return new Response("argv is a list", { status: 400 });
        return Response.json(await sandbox.exec(name, command));
      }
      if (action === "spawn" && request.method === "GET") {
        url.searchParams.set("sandbox", name);
        return await sandbox.fetch(new Request(url, request));
      }
      if (action === undefined && request.method === "DELETE") {
        await sandbox.destroy();
        return new Response(null, { status: 204 });
      }
      return new Response("Method not allowed", { status: 405 });
    } catch (error) {
      console.error({ event: "lorca.sandbox.failed", name, action, error: error instanceof Error ? error.stack : String(error) });
      return new Response(error instanceof Error ? error.message : "The sandbox failed", { status: 500 });
    }
  },
} satisfies ExportedHandler<Env>;
