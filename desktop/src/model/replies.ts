// Stands in for the CLI's agent loop in the demo, after the macOS app's ReplyEngine: bots work for
// a moment, run fake tools, hand off between each other, and post whole replies.

import { isGroup, newMessageID, providerName, type Bot, type Chat, type ToolInvocation } from "./models";
import type { AppStore } from "./store";

type Step =
  | { kind: "think"; botID: string; seconds: number }
  | { kind: "say"; botID: string; text: string }
  | { kind: "tool"; botID: string; tool: ToolInvocation; seconds: number }
  | { kind: "handoff"; from: string; to: string; reason: string };

/** A step after which a message typed during the turn takes over. */
const isSteeringBoundary = (step: Step) => step.kind !== "think";

const pick = <T>(items: T[]): T => items[Math.floor(Math.random() * items.length)]!;
const between = (low: number, high: number) => low + Math.random() * (high - low);

export class ReplyEngine {
  private tasks = new Map<string, { cancelled: boolean }>();
  private working = new Map<string, string>();
  /** Messages typed during a mock turn, promoted together at its next tool or answer boundary. */
  private steering = new Map<string, { prompt: string; chat: Chat }[]>();
  private turnCount = 0;

  constructor(private readonly store: AppStore) {}

  isRunning(chatID: string): boolean {
    return this.tasks.has(chatID);
  }

  cancel(chatID: string): void {
    const task = this.tasks.get(chatID);
    if (task) task.cancelled = true;
    this.tasks.delete(chatID);
    this.steering.delete(chatID);
    this.setWorking(undefined, chatID);
  }

  private setWorking(botID: string | undefined, chatID: string): void {
    const previous = this.working.get(chatID);
    if (previous && previous !== botID) this.store.setMockWorking(previous, chatID, false);
    if (botID) {
      this.working.set(chatID, botID);
      this.store.setMockWorking(botID, chatID, true);
    } else {
      this.working.delete(chatID);
    }
  }

  respond(prompt: string, chat: Chat): void {
    if (this.tasks.has(chat.id)) {
      this.steering.set(chat.id, [...(this.steering.get(chat.id) ?? []), { prompt, chat }]);
      return;
    }
    this.start(prompt, chat);
  }

  private start(prompt: string, chat: Chat): void {
    const script = this.makeScript(prompt, chat);
    if (script.length === 0) return;
    this.turnCount += 1;
    const task = { cancelled: false };
    this.tasks.set(chat.id, task);
    void (async () => {
      let steps = script;
      while (steps.length > 0) {
        if (task.cancelled) break;
        const step = steps.shift()!;
        await this.run(step, chat.id, task);
        const steered = isSteeringBoundary(step) ? this.takeSteering(chat.id) : undefined;
        if (steered) {
          this.turnCount += 1;
          steps = this.makeScript(steered.prompt, steered.chat);
        }
      }
      if (!task.cancelled) this.finish(chat.id);
    })();
  }

  private takeSteering(chatID: string): { prompt: string; chat: Chat } | undefined {
    const queued = this.steering.get(chatID);
    this.steering.delete(chatID);
    const last = queued?.[queued.length - 1];
    if (!queued || !last) return undefined;
    return { prompt: queued.map((entry) => entry.prompt).join("\n"), chat: last.chat };
  }

  private finish(chatID: string): void {
    this.tasks.delete(chatID);
    this.setWorking(undefined, chatID);
  }

  private async run(step: Step, chatID: string, task: { cancelled: boolean }): Promise<void> {
    switch (step.kind) {
      case "think":
        this.setWorking(step.botID, chatID);
        await sleep(step.seconds);
        return;
      case "say":
        this.setWorking(step.botID, chatID);
        await sleep(step.text.length * 0.004);
        if (task.cancelled) return;
        this.store.append(
          { id: newMessageID(), author: { kind: "bot", botID: step.botID }, body: { kind: "text", text: step.text }, state: { kind: "complete" }, createdAt: Date.now(), attachments: [] },
          chatID,
        );
        this.setWorking(undefined, chatID);
        return;
      case "tool": {
        this.setWorking(step.botID, chatID);
        const id = this.store.append(
          {
            id: newMessageID(),
            author: { kind: "bot", botID: step.botID },
            body: { kind: "tool", tool: { ...step.tool, isRunning: true } },
            state: { kind: "streaming" },
            createdAt: Date.now(),
            attachments: [],
          },
          chatID,
        );
        if (!id) return;
        await sleep(step.seconds);
        this.store.update(id, chatID, (message) => ({ ...message, body: { kind: "tool", tool: { ...step.tool, isRunning: false } }, state: { kind: "complete" } }));
        this.store.refreshChatList();
        return;
      }
      case "handoff":
        this.store.append(
          {
            id: newMessageID(),
            author: { kind: "bot", botID: step.from },
            body: { kind: "handoff", from: step.from, to: step.to, reason: step.reason },
            state: { kind: "complete" },
            createdAt: Date.now(),
            attachments: [],
          },
          chatID,
        );
        await sleep(0.6);
        return;
    }
  }

  private makeScript(prompt: string, chat: Chat): Step[] {
    const members = this.store.botsIn(chat);
    if (members.length === 0) return [];
    const lowered = prompt.toLowerCase();
    const mentioned = members.filter((bot) => lowered.includes(`@${bot.name.toLowerCase()}`));
    const everyone = lowered.includes("@everyone");
    const responders = everyone ? members : mentioned.length === 0 ? [members[0]!] : mentioned;
    const lead = responders[0];
    if (!lead) return [];

    // A bot on an offline Runner cannot run its turn; the envelope waits on the relay.
    const host = this.store.device(lead.runnerID);
    if (host && host.status === "offline") {
      return [
        { kind: "think", botID: lead.id, seconds: 0.5 },
        { kind: "say", botID: lead.id, text: `I'm queued on the relay — ${host.name} is offline, so this turn runs when that Runner reconnects.` },
      ];
    }

    const script: Step[] = [{ kind: "think", botID: lead.id, seconds: between(0.5, 1.1) }];
    const wantsWork = prompt.length > 46 || !prompt.includes("?");
    const helper = members.find((bot) => bot.id !== lead.id && this.store.device(bot.runnerID)?.status !== "offline");
    if (isGroup(chat) && wantsWork && helper && this.turnCount % 2 === 0) {
      script.push({
        kind: "tool",
        botID: lead.id,
        tool: { name: "list_teammates", summary: `Listed ${members.length} teammates`, detail: this.teammateDetail(members), isRunning: false },
        seconds: 0.7,
      });
      script.push({ kind: "handoff", from: lead.id, to: helper.id, reason: handoffReason(helper) });
      script.push({ kind: "think", botID: helper.id, seconds: 0.5 });
      script.push({ kind: "say", botID: helper.id, text: reply(helper, prompt) });
      script.push({ kind: "say", botID: lead.id, text: summary(helper) });
    } else {
      script.push({ kind: "say", botID: lead.id, text: reply(lead, prompt) });
    }
    if (everyone) {
      for (const bot of members.slice(1)) {
        if (bot.id === lead.id) continue;
        script.push({ kind: "think", botID: bot.id, seconds: 0.4 });
        script.push({ kind: "say", botID: bot.id, text: reply(bot, prompt) });
      }
    }
    return script;
  }

  private teammateDetail(members: Bot[]): string {
    const rows = members.map((bot) => {
      const host = this.store.device(bot.runnerID);
      const status = host?.status === "offline" ? "offline" : providerName(bot.provider);
      return `  { "name": "${bot.name}", "runner": "${host?.name ?? "?"}", "provider": "${status}" }`;
    });
    return `{\n  "teammates": [\n${rows.join(",\n")}\n  ]\n}`;
  }
}

function sleep(seconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, seconds * 1000));
}

function handoffReason(bot: Bot): string {
  switch (bot.id) {
    case "bot-patch":
      return "Write the change";
    case "bot-scout":
      return "Pull the context first";
    case "bot-quill":
      return "Say it in plain words";
    case "bot-ember":
      return "Check what is deployed";
    default:
      return "Take the next step";
  }
}

function summary(bot: Bot): string {
  return pick([
    `That matches what I expected from ${bot.name}. I'll keep the thread here until you say otherwise.`,
    `${bot.name} has it. Tell me if you want that turned into a task on another Runner.`,
    "Good — that closes the loop. Anything you want me to push back on?",
  ]);
}

function reply(bot: Bot, prompt: string): string {
  const lowered = prompt.toLowerCase();
  if (lowered.includes("hello") || lowered.includes("hi ") || lowered === "hi") return "Here. Ready when you are.";
  switch (bot.id) {
    case "bot-patch":
      return pick([
        'Smallest version that works:\n\n```rust\npub async fn serve(addr: SocketAddr) -> Result<()> {\n    let listener = TcpListener::bind(addr).await?;\n    tracing::info!(%addr, "lorca serve");\n    while let Ok((stream, _)) = listener.accept().await {\n        tokio::spawn(handle(stream));\n    }\n    Ok(())\n}\n```\n\nOne task per connection, and `handle` owns the decrypt step so the accept loop stays dumb.',
        "I'd keep this in the CLI rather than the app. The app should stay a renderer — the moment it knows how to decrypt, the key material has two homes and the threat model gets harder to explain.",
        "Two options, and they are not close:\n\n- Put it behind `bootstrap` and let the app render whatever comes back.\n- Add a new event kind and teach both sides about it.\n\nThe first one is free. Take the first one.",
      ]);
    case "bot-scout":
      return pick([
        "Checked the tree. The only place that touches this is the `blobs` handler and one test fixture, so the change is contained. Nothing in `web/` reads it.",
        "Relevant prior art: Happy wraps the account DEK to each machine public key at pairing time, which is what `ARCHITECTURE.md` already describes. Following it means recovery is the backup phrase and nothing else, which is the property you want.",
        "I found two answers and they disagree. The schema says `seq` is unique per identity; the handler treats it as unique per `(identity, kind)`. Worth deciding before the first migration lands, because it is painful afterwards.",
      ]);
    case "bot-quill":
      return pick([
        'Draft: "Your bots run on computers you own. Assign one to a Runner, and it works there with your account\'s encrypted provider credentials. The relay carries ciphertext and nothing else."\n\nThree sentences, no adjectives doing work they haven\'t earned.',
        'I\'d cut "seamlessly" and "powerful". They are the words people skim. What is left says the same thing and is shorter.',
      ]);
    case "bot-ember":
      return pick([
        "Blast radius first: this touches the Worker only, no D1 migration, so a bad deploy is a rollback and not a restore.",
        "Deployed. The relay is answering the challenge endpoint in about 40ms from here, which is the number to watch when we add the blob listing.",
      ]);
    default:
      return pick([
        "Here's how I'd sequence it:\n\n- Get the local websocket answering `bootstrap` with the same shape this app already renders.\n- Then swap the mock store for those events, one screen at a time.\n- Pairing last, because it is the only part that needs two machines to test.\n\nWant me to hand the first piece to Developer?",
        "The constraint that decides this is **a bot runs on its assigned Runner**. Its files and plugins are there, so the answer is a job envelope, not a call from here.",
        "Short answer: yes. Longer answer: yes, but not until pairing works on two machines, because that is where this gets interesting.",
      ]);
  }
}
