// What the CLI sends, as the app reads it: custom providers in the account's provider statuses and
// as a bot's provider, and a command's card.

import { expect, test } from "bun:test";
import { runsInForeground } from "./models";
import { toBot, toCustomModel, toMessage, toProviders, type WireBot } from "./wire";

test("statuses keep custom providers after the built-in ones and leave out unknown kinds", () => {
  const providers = toProviders([
    { kind: "deepseek", is_connected: true, detail: "sk-live…4f2c" },
    { kind: "mistral", is_connected: true, detail: "sk-…0000" },
    {
      kind: "custom:openrouter",
      is_connected: true,
      detail: "sk-…abcd · https://openrouter.ai/api/v1",
      base_url: "https://openrouter.ai/api/v1",
      name: "OpenRouter",
      api: "chat-completions",
      models: [{ id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5", context_window: 1_000_000, max_output: 128_000, images: true }, { id: "qwen3:8b" }],
    },
    { kind: "custom:lab", is_connected: true, detail: "http://lab:8000", base_url: "http://lab:8000", name: "Lab", api: "realtime" },
  ]);
  expect(providers.map((provider) => provider.kind)).toEqual(["deepseek", "custom:openrouter", "custom:lab"]);
  expect(providers[0]).toEqual({ kind: "deepseek", isConnected: true, detail: "sk-live…4f2c" });
  expect(providers[1]).toEqual({
    kind: "custom:openrouter",
    isConnected: true,
    detail: "sk-…abcd · https://openrouter.ai/api/v1",
    baseURL: "https://openrouter.ai/api/v1",
    name: "OpenRouter",
    api: "chat-completions",
    models: [
      { id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5", contextWindow: 1_000_000, maxOutput: 128_000, images: true },
      { id: "qwen3:8b" },
    ],
  });
  // An API this build does not know is left for the sheet to pick; no models is an empty list.
  expect(providers[2]).toMatchObject({ name: "Lab", api: undefined, models: [] });
  expect(toProviders(null)).toEqual([]);
});

test("a listed model keeps what its server said of it", () => {
  expect(toCustomModel({ id: "llava:13b", name: null, context_window: 4_096, max_output: null, images: true })).toEqual({ id: "llava:13b", contextWindow: 4_096, images: true });
});

test("a bot keeps its custom provider, and one this build does not know runs as DeepSeek", () => {
  const wire: WireBot = {
    id: "bot-lab",
    name: "Lab",
    description: "",
    symbol_name: "sparkles",
    accent: "blue",
    runner_id: "dev-workbench",
    provider: "custom:openrouter",
    model: "qwen3:8b",
    thinking: "low",
    created_at: 1,
  };
  expect(toBot(wire)).toMatchObject({ provider: "custom:openrouter", model: "qwen3:8b", thinking: "low" });
  expect(toBot({ ...wire, provider: "custom:gone", model: null, thinking: "" })).toMatchObject({ provider: "custom:gone", model: undefined, thinking: undefined });
  expect(toBot({ ...wire, provider: "mistral" }).provider).toBe("deepseek");
});

test("Run in Background is offered while the bot's call waits on a command that is not in the background yet", () => {
  const row = (isRunning: boolean, run: Record<string, unknown>) =>
    toMessage({
      id: "m1",
      chat_id: "c1",
      author: { kind: "bot", bot_id: "b1" },
      body: { kind: "tool", name: "bash", summary: "Running", is_running: isRunning, run: { session_id: "bash-1a2b3c", command: "npm run dev", state: "running", ...run } },
      state: { kind: "complete" },
      created_at: 1,
    });
  const waitedOn = row(true, {});
  expect(waitedOn.body.kind === "tool" && waitedOn.body.tool.run?.background).toBe(false);
  expect(runsInForeground(waitedOn)).toBe(true);
  const sent = row(false, { background: true });
  expect(sent.body.kind === "tool" && sent.body.tool.run?.background).toBe(true);
  expect(runsInForeground(sent)).toBe(false);
  // Started in the background, during its first two seconds.
  expect(runsInForeground(row(true, { background: true }))).toBe(false);
  // Its call returned; the bot is no longer waiting on it.
  expect(runsInForeground(row(false, {}))).toBe(false);
  expect(runsInForeground(row(true, { state: "checking", session_id: null }))).toBe(false);
});
