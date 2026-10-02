// The parts of the phone's core that run outside React Native, run with `bun test`: the
// pairing-string check, the model helpers, and the formatting the transcript and list share.

import { describe, expect, test } from "bun:test";
import {
  addModelRow,
  attachmentSummary,
  connectedProviders,
  contextWindowLabel,
  CUSTOM_PRESETS,
  customAPI,
  customPreset,
  customProviderNamed,
  customRequestURL,
  defaultModelId,
  defaultProviderName,
  fileSize,
  filterModelRows,
  isCustomProvider,
  isHTTPURL,
  isLoopbackHost,
  isProviderKind,
  isSentMessage,
  mergeListedModels,
  modelIdToAdd,
  modelLabel,
  modelListingNote,
  presetForURL,
  providerConnectMethod,
  providerDefaultBaseURL,
  providerKinds,
  providerLabel,
  providerModels,
  providerUsesAPIKey,
  PROVIDER_KINDS,
  runsInTerminal,
  savedModelRows,
  selectedModelIds,
  showsCard,
  toggleModelRow,
  urlHost,
  thinkingLevels,
  withCustomModels,
  type ProviderModel,
  type Body,
  type Bot,
  type Chat,
  type CommandRun,
  type ProviderStatus,
} from "./model";
import { parsePairingString } from "./pairing";
import { daySeparator, joinDictation, preview, stamp, time, workingActivity, type WorkState } from "../ui/format";

let nextId = 0;
const uuid = () => `m-${++nextId}`;

describe("pairing", () => {
  test("parses a pairing string from another Device", () => {
    const target = parsePairingString(
      " lorca://pair?relay=http%3A%2F%2F127.0.0.1%3A18790%2F&id=Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs&ek=nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw&n=1qUeEODuTz_A2y5jSZdBqQ \n",
    );
    expect(target).toEqual({
      relay: "http://127.0.0.1:18790",
      id: "Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs",
      ek: "nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw",
      nonce: "1qUeEODuTz_A2y5jSZdBqQ",
    });
  });

  test("rejects other text", () => {
    expect(() => parsePairingString("hello")).toThrow("not a Lorca pairing string");
    expect(() => parsePairingString("lorca://pair?relay=x&id=y")).toThrow("missing a field");
  });
});

describe("model", () => {
  test("only a finished message_bot is a sent message", () => {
    const sent: Body = { kind: "tool", name: "message_bot", summary: "Messaged Scout", detail: "look into tea", is_running: false };
    const running: Body = { ...sent, is_running: true };
    const failed: Body = { ...sent, summary: "message_bot failed" };
    expect(isSentMessage(sent)).toBe(true);
    expect(isSentMessage(running)).toBe(false);
    expect(isSentMessage(failed)).toBe(false);
    expect(isSentMessage({ kind: "text", text: "Messaged Scout" })).toBe(false);
  });

  test("a command shows its card only while it needs the user", () => {
    const call = (is_running: boolean, state: CommandRun["state"], handed_over = false) =>
      showsCard({ kind: "tool", name: "bash", summary: "Running", detail: "", is_running, run: { command: "sudo pacman -Syu", state, handed_over } });
    // Auto-review's question, whether or not the call still waits on it.
    expect(call(true, "asking")).toBe(true);
    // Inside its call: the working row says what runs.
    expect(call(true, "checking")).toBe(false);
    expect(call(true, "running")).toBe(false);
    // Its call returned, and the bot is still dealing with it: it may answer the question itself.
    expect(call(false, "waiting")).toBe(false);
    expect(call(false, "running")).toBe(false);
    // Handed over: waiting for the user's answer, or going on by itself after the turn.
    expect(call(false, "waiting", true)).toBe(true);
    expect(call(false, "running", true)).toBe(true);
    // Ended.
    for (const state of ["exited", "failed", "stopped", "denied", "expired", "dismissed"] as const) expect(call(false, state, true)).toBe(false);
    expect(showsCard({ kind: "tool", name: "read", summary: "", detail: "", is_running: false })).toBe(false);
  });

  test("a command runs in its terminal from its session's start to its end", () => {
    const row = (state: CommandRun["state"], session_id?: string) => ({
      id: "call", chat_id: "chat", author: { kind: "bot" as const, bot_id: "bot" }, state: { kind: "streaming" as const }, created_at: 1,
      body: { kind: "tool" as const, name: "bash", summary: "Running", detail: "", is_running: true, run: { command: "bun install", state, session_id } },
    });
    // Auto-review judges it, or asks, before a terminal runs it.
    for (const state of ["running", "checking", "asking"] as const) expect(runsInTerminal(row(state))).toBe(false);
    expect(runsInTerminal(row("running", "bash-1"))).toBe(true);
    expect(runsInTerminal(row("waiting", "bash-1"))).toBe(true);
    for (const state of ["exited", "failed", "stopped"] as const) expect(runsInTerminal(row(state, "bash-1"))).toBe(false);
    expect(runsInTerminal(undefined)).toBe(false);
  });

  test("describes provider credential setup", () => {
    expect(isProviderKind("opencode-go")).toBe(true);
    expect(isProviderKind("unknown")).toBe(false);
    expect(providerUsesAPIKey("anthropic")).toBe(true);
    expect(providerUsesAPIKey("chatgpt")).toBe(false);
    expect(providerConnectMethod("opencode-go")).toBe("providers.connect_opencode_go");
    expect(providerConnectMethod("grok")).toBe("providers.connect_grok");
    expect(providerDefaultBaseURL("deepseek")).toBe("https://api.deepseek.com");
    expect(providerDefaultBaseURL("chatgpt")).toBe("");
  });

  test("offers the models and thinking levels the catalog lists", () => {
    // As the core's snapshot carries them, in the catalog's order.
    const catalog: ProviderModel[] = [
      { provider: "anthropic", id: "claude-opus-5", name: "Claude Opus 5", levels: ["off", "low", "medium", "high", "xhigh", "max"] },
      { provider: "anthropic", id: "claude-opus-5-5", name: "Claude Opus 5.5", levels: ["low", "medium", "high", "xhigh", "max"] },
      { provider: "anthropic", id: "claude-haiku-4-5", name: "Claude Haiku 4.5", levels: ["off", "minimal", "low", "medium", "high"] },
      { provider: "opencode", id: "kimi-k3", name: "Kimi K3", levels: ["max"] },
      { provider: "opencode", id: "big-pickle", name: "Big Pickle", levels: [] },
    ];
    expect(providerModels(catalog, "anthropic").map((m) => m.id)).toEqual(["claude-opus-5", "claude-opus-5-5", "claude-haiku-4-5"]);
    expect(providerModels(catalog, "grok")).toEqual([]);
    // The default model's levels, until the bot picks one.
    expect(thinkingLevels(catalog, "anthropic", undefined)).toEqual(["off", "low", "medium", "high", "xhigh", "max"]);
    expect(thinkingLevels(catalog, "anthropic", "claude-opus-5-5")).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(thinkingLevels(catalog, "opencode", "kimi-k3")).toEqual(["max"]);
    expect(thinkingLevels(catalog, "opencode", "big-pickle")).toEqual([]);
    // A model the catalog does not have gets every level the provider's models take.
    expect(thinkingLevels(catalog, "anthropic", "claude-custom")).toEqual(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);
  });
});

describe("custom providers", () => {
  // The statuses as the core lists them: the six built-ins, then custom providers in the order added.
  const builtIn: ProviderStatus[] = PROVIDER_KINDS.map((kind) => ({ kind, is_connected: kind === "deepseek", detail: kind === "deepseek" ? "sk-…abcd" : "Not connected" }));
  const openrouter: ProviderStatus = {
    kind: "custom:openrouter",
    is_connected: true,
    detail: "sk-…abcd · https://openrouter.ai/api/v1",
    base_url: "https://openrouter.ai/api/v1",
    name: "OpenRouter",
    api: "chat-completions",
    models: [
      { id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5", context_window: 1_000_000, max_output: 128_000, images: true, levels: ["off", "low", "medium", "high", "xhigh", "max"] },
      { id: "qwen3:8b", levels: ["low", "medium", "high"] },
    ],
  };
  const lab: ProviderStatus = { kind: "custom:lab", is_connected: true, detail: "http://192.168.1.20:11434/v1", base_url: "http://192.168.1.20:11434/v1", name: "Lab", api: "responses", models: [{ id: "llama4", name: "", levels: ["low", "medium", "high"] }] };
  const statuses = [...builtIn, openrouter, lab];

  test("kinds start with custom:", () => {
    expect(isCustomProvider("custom:openrouter")).toBe(true);
    expect(isCustomProvider("anthropic")).toBe(false);
    expect(isProviderKind("custom:openrouter")).toBe(false);
  });

  test("a custom provider goes by its name, or its slug once it is gone", () => {
    expect(providerLabel("custom:openrouter", statuses)).toBe("OpenRouter");
    expect(providerLabel("custom:gone", statuses)).toBe("gone");
    expect(providerLabel("custom:openrouter", [])).toBe("openrouter");
    // Built-ins keep their names, with or without statuses.
    expect(providerLabel("opencode-go", statuses)).toBe("OpenCode Go");
    expect(providerLabel("anthropic", [])).toBe("Anthropic");
  });

  // The core's catalog, as the snapshot carries it, with the custom providers' models after it.
  const catalog = withCustomModels([{ provider: "chatgpt", id: "gpt-6.1-sol", name: "GPT-6.1 Sol", levels: ["low", "medium", "high", "xhigh", "max"] }], statuses);

  test("its models join the catalog, named as its server lists them, the first the default", () => {
    expect(providerModels(catalog, "custom:openrouter").map((model) => [model.id, model.name])).toEqual([
      ["anthropic/claude-sonnet-5", "Anthropic: Claude Sonnet 5"],
      ["qwen3:8b", "qwen3:8b"],
    ]);
    expect(providerModels(catalog, "custom:lab").map((model) => model.name)).toEqual(["llama4"]);
    expect(providerModels(catalog, "custom:gone")).toEqual([]);
    expect(providerModels(catalog, "chatgpt").map((model) => model.id)).toEqual(["gpt-6.1-sol"]);
  });

  test("each of its models takes the thinking levels the core gives it", () => {
    expect(thinkingLevels(catalog, "custom:openrouter", undefined)).toEqual(["off", "low", "medium", "high", "xhigh", "max"]);
    expect(thinkingLevels(catalog, "custom:openrouter", "qwen3:8b")).toEqual(["low", "medium", "high"]);
    expect(thinkingLevels(catalog, "custom:lab", "llama4")).toEqual(["low", "medium", "high"]);
  });

  test("custom providers come after the built-ins, in the order added", () => {
    expect(providerKinds(statuses)).toEqual([...PROVIDER_KINDS, "custom:openrouter", "custom:lab"]);
    expect(providerKinds([])).toEqual([...PROVIDER_KINDS]);
    expect(connectedProviders(statuses)).toEqual(["deepseek", "custom:openrouter", "custom:lab"]);
  });

  test("the base URL becomes the URL the core calls", () => {
    expect(customRequestURL("messages", "https://api.anthropic.com/v1")).toBe("https://api.anthropic.com/v1/messages");
    expect(customRequestURL("messages", "https://api.anthropic.com/v1/messages")).toBe("https://api.anthropic.com/v1/messages");
    expect(customRequestURL("messages", " https://api.anthropic.com/ ")).toBe("https://api.anthropic.com/v1/messages");
    expect(customRequestURL("chat-completions", "http://localhost:11434/v1/chat/completions")).toBe("http://localhost:11434/v1/chat/completions");
    expect(customRequestURL("chat-completions", "https://openrouter.ai/api/v1")).toBe("https://openrouter.ai/api/v1/chat/completions");
    expect(customRequestURL("responses", "https://x/v1/")).toBe("https://x/v1/responses");
    expect(customRequestURL("responses", "https://x/v1/responses//")).toBe("https://x/v1/responses");
    // One endpoint is cut, and only the protocol's own.
    expect(customRequestURL("chat-completions", "https://x/v1/responses")).toBe("https://x/v1/responses/chat/completions");
    expect(customRequestURL("chat-completions", "  ")).toBe("");
  });

  test("protocols name their path and base URL example", () => {
    expect(customAPI("messages")).toMatchObject({ title: "Anthropic Messages", path: "/v1/messages", placeholder: "https://api.example.com" });
    expect(customAPI("responses")).toMatchObject({ title: "OpenAI Responses", path: "/responses", placeholder: "https://api.example.com/v1" });
    // A protocol this build does not know reads as the first.
    expect(customAPI(undefined).id).toBe("chat-completions");
  });

  test("presets start the form from a server people often add", () => {
    expect(CUSTOM_PRESETS.map((preset) => [preset.name, preset.api, preset.baseURL])).toEqual([
      ["OpenAI", "responses", "https://api.openai.com/v1"],
      ["OpenRouter", "chat-completions", "https://openrouter.ai/api/v1"],
      ["Gemini", "chat-completions", "https://generativelanguage.googleapis.com/v1beta/openai"],
      ["Groq", "chat-completions", "https://api.groq.com/openai/v1"],
      ["Together AI", "chat-completions", "https://api.together.xyz/v1"],
      ["Ollama", "chat-completions", "http://localhost:11434/v1"],
      ["LM Studio", "chat-completions", "http://localhost:1234/v1"],
    ]);
    expect(CUSTOM_PRESETS.filter((preset) => preset.local).map((preset) => preset.name)).toEqual(["Ollama", "LM Studio"]);
    expect(customPreset("together ai")?.keyPlaceholder()).toBe("Key from api.together.ai");
    expect(customPreset("Lab")).toBeUndefined();
    expect(customPreset(undefined)).toBeUndefined();
    // A preset the account has already is that provider, whatever the case of its name.
    expect(customProviderNamed("openrouter", statuses)?.kind).toBe("custom:openrouter");
    expect(customProviderNamed("Groq", statuses)).toBeUndefined();
    expect(customProviderNamed("DeepSeek", statuses)).toBeUndefined();
  });

  test("a base URL names its host, which names a provider left unnamed", () => {
    expect(urlHost("https://openrouter.ai/api/v1")).toBe("openrouter.ai");
    expect(urlHost(" http://192.168.1.20:11434/v1 ")).toBe("192.168.1.20:11434");
    expect(urlHost("https://user:secret@Gateway.Example.com?x=1")).toBe("gateway.example.com");
    expect(urlHost("http://[::1]:8080/v1")).toBe("[::1]:8080");
    expect(urlHost("openrouter.ai/api/v1")).toBe("");
    expect(urlHost("")).toBe("");
    expect(isHTTPURL("https://api.groq.com/openai/v1")).toBe(true);
    expect(isHTTPURL("HTTP://localhost:1234")).toBe(true);
    expect(isHTTPURL("https://")).toBe(false);
    expect(isHTTPURL("ftp://example.com")).toBe(false);
    expect(isHTTPURL("api.openai.com/v1")).toBe(false);
    for (const host of ["localhost:11434", "127.0.0.1:1234", "[::1]:8080", "0.0.0.0", "ollama.localhost"]) expect(isLoopbackHost(host)).toBe(true);
    for (const host of ["192.168.1.20:11434", "api.openai.com", "localhost.example.com", ""]) expect(isLoopbackHost(host)).toBe(false);
  });

  test("an unnamed provider takes the name of the preset whose server it names, else the host", () => {
    expect(presetForURL("http://localhost:11434/v1")?.name).toBe("Ollama");
    expect(presetForURL("http://localhost:1234")?.name).toBe("LM Studio");
    expect(presetForURL("http://localhost:8080/v1")).toBeUndefined();
    expect(defaultProviderName("http://localhost:11434/v1")).toBe("Ollama");
    expect(defaultProviderName("https://openrouter.ai/api/v1/chat/completions")).toBe("OpenRouter");
    expect(defaultProviderName("https://generativelanguage.googleapis.com/v1beta/openai")).toBe("Gemini");
    expect(defaultProviderName("http://192.168.1.20:11434/v1")).toBe("192.168.1.20:11434");
    expect(defaultProviderName("")).toBe("");
  });

  test("context windows read the short way", () => {
    expect(contextWindowLabel(128_000)).toBe("128K");
    expect(contextWindowLabel(131_072)).toBe("128K");
    expect(contextWindowLabel(200_000)).toBe("200K");
    expect(contextWindowLabel(32_768)).toBe("32K");
    expect(contextWindowLabel(262_144)).toBe("256K");
    expect(contextWindowLabel(1_000_000)).toBe("1M");
    expect(contextWindowLabel(1_048_576)).toBe("1M");
    expect(contextWindowLabel(1_047_576)).toBe("1M");
    expect(contextWindowLabel(1_500_000)).toBe("1.5M");
    expect(contextWindowLabel(999_999)).toBe("1M");
    expect(contextWindowLabel(512)).toBe("512");
  });

  test("a listing joins the rows: the user's first, then the server's in its order", () => {
    const saved = savedModelRows([{ id: "qwen3:8b" }, { id: "mystery" }]);
    expect(saved).toEqual([
      { id: "qwen3:8b", selected: true, source: "user" },
      { id: "mystery", selected: true, source: "user" },
    ]);
    const listing = [{ id: "llama4", name: "Llama 4", context_window: 131_072 }, { id: "qwen3:8b", name: "Qwen3 8B", images: false }, { id: "llama4" }];
    const merged = mergeListedModels(saved, listing);
    // Saved rows stay picked and first, with what the server says of them; the rest come unpicked, once.
    expect(merged).toEqual([
      { id: "qwen3:8b", name: "Qwen3 8B", images: false, selected: true, source: "user" },
      { id: "mystery", selected: true, source: "user" },
      { id: "llama4", name: "Llama 4", context_window: 131_072, selected: false, source: "server" },
    ]);
    // A new list keeps what was picked from the last one, drops the rest of it, and adds its own.
    const picked = toggleModelRow(merged, "llama4");
    const next = mergeListedModels(toggleModelRow(picked, "mystery"), [{ id: "gpt-6-sol" }]);
    expect(next.map((row) => [row.id, row.selected, row.source])).toEqual([
      ["qwen3:8b", true, "user"],
      ["mystery", false, "user"],
      ["llama4", true, "server"],
      ["gpt-6-sol", false, "server"],
    ]);
    // A server that lists nothing leaves the user's rows and the picked ones.
    expect(mergeListedModels(next, []).map((row) => row.id)).toEqual(["qwen3:8b", "mystery", "llama4"]);
  });

  test("a short list arriving with nothing picked is picked whole; a long one is not", () => {
    const short = mergeListedModels([], [{ id: "a" }, { id: "b" }]);
    expect(short.map((row) => row.selected)).toEqual([true, true]);
    const long = mergeListedModels([], Array.from({ length: 9 }, (_, n) => ({ id: `m${n}` })));
    expect(long.some((row) => row.selected)).toBe(false);
    // Something picked already: the list joins unpicked.
    const added = addModelRow([], "my-model");
    expect(mergeListedModels(added, [{ id: "a" }]).map((row) => [row.id, row.selected])).toEqual([
      ["my-model", true],
      ["a", false],
    ]);
  });

  test("the search filters by id or name, and offers to add an id no row has", () => {
    const rows = mergeListedModels([], [{ id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5" }, { id: "qwen/qwen3-coder", name: "Qwen: Qwen3 Coder" }]);
    expect(filterModelRows(rows, "CLAUDE").map((row) => row.id)).toEqual(["anthropic/claude-sonnet-5"]);
    expect(filterModelRows(rows, "qwen3").map((row) => row.id)).toEqual(["qwen/qwen3-coder"]);
    expect(filterModelRows(rows, " ")).toHaveLength(2);
    expect(modelIdToAdd(rows, "  my-finetune ")).toBe("my-finetune");
    expect(modelIdToAdd(rows, "qwen/qwen3-coder")).toBeNull();
    expect(modelIdToAdd(rows, "Qwen/Qwen3-Coder")).toBe("Qwen/Qwen3-Coder");
    expect(modelIdToAdd(rows, "   ")).toBeNull();
    // An added id goes to the top, picked.
    expect(addModelRow(rows, "my-finetune").map((row) => [row.id, row.selected, row.source])).toEqual([
      ["my-finetune", true, "user"],
      ["anthropic/claude-sonnet-5", true, "server"],
      ["qwen/qwen3-coder", true, "server"],
    ]);
  });

  test("the picked ids go out with the default first", () => {
    const rows = toggleModelRow(mergeListedModels([], [{ id: "a" }, { id: "b" }, { id: "c" }]), "b");
    expect(defaultModelId(rows)).toBe("a");
    expect(selectedModelIds(rows)).toEqual(["a", "c"]);
    expect(defaultModelId(rows, "c")).toBe("c");
    expect(selectedModelIds(rows, "c")).toEqual(["c", "a"]);
    // A chosen default no longer picked gives way to the first picked.
    expect(selectedModelIds(rows, "b")).toEqual(["a", "c"]);
    expect(selectedModelIds(toggleModelRow(toggleModelRow(rows, "a"), "c"), "c")).toEqual([]);
    expect(defaultModelId([], "a")).toBeUndefined();
    expect(modelLabel({ id: "qwen3:8b", name: " " })).toBe("qwen3:8b");
  });

  test("the form says where the server's list stands", () => {
    expect(modelListingNote({ state: "none" })).toBe("Enter the base URL to load the server’s models.");
    expect(modelListingNote({ state: "loading" })).toBe("Loading models…");
    expect(modelListingNote({ state: "unlisted" })).toBe("This server doesn’t list its models. Add model IDs in Models.");
    expect(modelListingNote({ state: "error", message: "Lab rejected that key" })).toBe("Lab rejected that key. You can still add model IDs in Models.");
    expect(modelListingNote({ state: "error", message: "Lab did not answer like an API at http://lab. Check the base URL." })).toBe("Lab did not answer like an API at http://lab. Check the base URL. You can still add model IDs in Models.");
    expect(modelListingNote({ state: "listed" })).toBeUndefined();
  });
});

describe("attachments", () => {
  const photo = { id: "att-1", name: "IMG_1.jpg", mime: "image/jpeg", size: 2_500_000 };
  const doc = { id: "att-2", name: "report.pdf", mime: "application/pdf", size: 900 };

  test("summaries and sizes", () => {
    expect(attachmentSummary([])).toBe("");
    expect(attachmentSummary([photo])).toBe("Photo");
    expect(attachmentSummary([photo, photo])).toBe("2 photos");
    expect(attachmentSummary([doc])).toBe("report.pdf");
    expect(attachmentSummary([photo, doc])).toBe("2 files");
    expect(fileSize(900)).toBe("900 B");
    expect(fileSize(2_500_000)).toBe("2.4 MB");
  });

  test("dictation joins after typed text", () => {
    expect(joinDictation("", "hello there")).toBe("hello there");
    expect(joinDictation("Hi", "there")).toBe("Hi there");
    expect(joinDictation("Hi ", "there")).toBe("Hi there");
    expect(joinDictation("Hi", "")).toBe("Hi");
  });
});

describe("format", () => {
  const bots = new Map<string, Bot>([
    ["b1", { id: "b1", name: "Chef", description: "", symbol_name: "sparkles", accent: "indigo", runner_id: "r", provider: "deepseek", created_at: 0 }],
    ["b2", { id: "b2", name: "Scout", description: "", symbol_name: "magnifyingglass", accent: "teal", runner_id: "r", provider: "deepseek", created_at: 0 }],
  ]);
  const chat = (kind: "dm" | "group", messages: Chat["messages"]): Chat => ({ id: "c", kind, bot_ids: ["b1", "b2"], is_pinned: false, created_at: 0, messages, unread_count: 0 });
  const msg = (author: Chat["messages"][number]["author"], body: Body): Chat["messages"][number] => ({ id: uuid(), chat_id: "c", author, body, state: { kind: "complete" }, created_at: 1 });

  test("previews the way the desktop sidebar does", () => {
    expect(preview(chat("dm", []), bots)).toBe("No messages yet");
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "hi\nthere **now**" })]), bots)).toBe("hi there now");
    const photo = { id: "att-1", name: "IMG_1.jpg", mime: "image/jpeg", size: 1 };
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "", attachments: [photo] })]), bots)).toBe("Photo");
    expect(preview(chat("group", [msg({ kind: "bot", bot_id: "b2" }, { kind: "text", text: "yes" })]), bots)).toBe("Scout: yes");
    const tool = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "read", summary: "Read a file", detail: "x", is_running: false });
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "go" }), tool]), bots)).toBe("go");
    const sent = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "message_bot", summary: "Messaged Scout", detail: "please look", is_running: false, target_bot_id: "b2" });
    expect(preview(chat("dm", [sent]), bots)).toBe("Messaged Scout: please look");
    const handoff = msg({ kind: "bot", bot_id: "b1" }, { kind: "handoff", from: "b1", to: "b2", reason: "over to you" });
    expect(preview({ ...chat("dm", [handoff]), bot_ids: ["b2"] }, bots)).toBe("Message from Chef: over to you");
  });

  test("the working row reads what the one bot at work is doing", () => {
    const you = msg({ kind: "you" }, { kind: "text", text: "set it up" });
    const tool = (name: string, extra: Partial<Extract<Body, { kind: "tool" }>> = {}) =>
      msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name, summary: `Running ${name}…`, detail: "", is_running: true, ...extra });
    const state = (messages: Chat["messages"], more: Partial<WorkState> = {}): WorkState => ({
      running: { job: { chatId: "c", botId: "b1" } },
      thinking: {},
      retries: {},
      chats: [chat("dm", messages)],
      bots: [...bots.values()],
      devices: [],
      ...more,
    });
    const activity = (messages: Chat["messages"], more?: Partial<WorkState>) => workingActivity(state(messages, more), "c");

    expect(activity([you])).toBeNull();
    expect(activity([you, tool("bash", { description: "Install dependencies" })])).toBe("Running command: Install dependencies…");
    // Between calls the row keeps the last one.
    expect(activity([you, tool("bash", { is_running: false })])).toBe("Running commands…");
    expect(activity([you, tool("memory_update")])).toBe("Taking a note…");
    expect(activity([you, tool("message_bot", { target_bot_id: "b2" })])).toBe("Messaging Scout…");
    expect(activity([you, tool("message_bot", { summary: "Messaged Scout", is_running: false })])).toBeNull();
    expect(activity([you, tool("github__create_issue")])).toBe("Using Github…");
    const runner = { id: "r", name: "Mac", model: "", os: "macos", os_version: "", machine_key: "", is_this_device: false, status: "online" as const, last_seen: 0, plugins: [{ id: "github", name: "GitHub", state: "ready" as const }] };
    expect(activity([you, tool("github__create_issue")], { devices: [runner] })).toBe("Using GitHub…");
    expect(activity([you, tool("routines")])).toBe("Working…");
    // A script reads as the plugin its latest call used, which the CLI puts in its description.
    expect(activity([you, tool("codemode")])).toBe("Working…");
    expect(activity([you, tool("codemode", { description: "Linear", is_running: false })])).toBe("Using Linear…");
    // Or as the command it runs, which the CLI names apart from the plugin.
    expect(activity([you, tool("codemode", { description: "Linear", script_command: "Run the tests" })])).toBe("Running command: Run the tests…");
    // What the bot said since is the news; its thinking and a retry outrank the last call.
    expect(activity([you, tool("read"), msg({ kind: "bot", bot_id: "b1" }, { kind: "text", text: "Done" })])).toBeNull();
    expect(activity([you, tool("read")], { thinking: { c: "b1" } })).toBe("Thinking…");
    expect(activity([you], { thinking: { c: "b1" }, retries: { c: { attempt: 2, max_attempts: 3, delay_ms: 3600 } } })).toBe("Retrying (2 of 3) in 4 s…");
    // Two bots at work read as their names.
    expect(activity([you, tool("read")], { running: { a: { chatId: "c", botId: "b1" }, b: { chatId: "c", botId: "b2" } } })).toBeNull();
  });

  test("stamps and separators", () => {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 16, 13);
    expect(stamp(today)).toBe(time(today));
    expect(time(today)).toBe("4:13 PM");
    expect(daySeparator(today)).toBe("Today 4:13 PM");
    const yesterday = new Date(today.getTime() - 86400_000);
    expect(stamp(yesterday)).toBe("Yesterday");
    expect(daySeparator(yesterday)).toBe("Yesterday 4:13 PM");
    const lastYear = new Date(now.getFullYear() - 1, 8, 2, 9, 0);
    expect(stamp(lastYear)).toBe(`9/2/${String(now.getFullYear() - 1).slice(-2)}`);
    expect(daySeparator(lastYear)).toMatch(/^\w{3}, Sep 2 9:00 AM$/);
  });
});
