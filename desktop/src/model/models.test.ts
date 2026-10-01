// Custom providers in the model: their kinds, names, models, and thinking levels; the presets and
// the base URL the custom provider sheet reads; and its model checklist.

import { expect, test } from "bun:test";
import {
  addCandidate,
  addModel,
  customBaseURLPlaceholder,
  customEndpoint,
  customEndpointNote,
  customHost,
  customPresets,
  filterModels,
  isCustomKind,
  isProviderKind,
  isUsableBaseURL,
  matchingPreset,
  modelDisplayName,
  orderedModelIDs,
  presetProvider,
  providerModels,
  providerName,
  providerSubtitle,
  providerSymbol,
  savedChecklist,
  suggestedProviderName,
  takeListing,
  thinkingLevels,
  withCustomModels,
  toggleModel,
  usesAPIKey,
  type CustomModel,
  type ModelChecklist,
  type ProviderCredential,
} from "./models";

const ollama: ProviderCredential = {
  kind: "custom:ollama",
  isConnected: true,
  detail: "http://localhost:11434/v1",
  baseURL: "http://localhost:11434/v1",
  name: "Ollama",
  api: "chat-completions",
  models: [
    { id: "qwen3:8b", levels: ["low", "medium", "high"] },
    { id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5", levels: ["off", "low", "medium", "high", "xhigh", "max"] },
  ],
};

test("a custom kind is a provider kind, and an unknown one is not", () => {
  expect(isProviderKind("deepseek")).toBe(true);
  expect(isProviderKind("custom:openrouter")).toBe(true);
  expect(isCustomKind("custom:openrouter")).toBe(true);
  expect(isCustomKind("deepseek")).toBe(false);
  expect(isProviderKind("mistral")).toBe(false);
});

test("a custom provider goes by its name, and by its slug once it is deleted", () => {
  expect(providerName("opencode")).toBe("OpenCode Zen");
  expect(providerName("opencode-go", [ollama])).toBe("OpenCode Go");
  expect(providerName("custom:ollama", [ollama])).toBe("Ollama");
  expect(providerName("custom:my-lab", [ollama])).toBe("my-lab");
});

test("a custom provider offers its saved models with the levels the CLI gives each", () => {
  const catalog = withCustomModels([{ provider: "grok", id: "grok-4.7", label: "Grok 4.7", levels: ["low", "medium", "high", "xhigh"] }], [ollama]);
  expect(providerModels(catalog, "custom:ollama")).toEqual([
    { provider: "custom:ollama", id: "qwen3:8b", label: "qwen3:8b", levels: ["low", "medium", "high"] },
    { provider: "custom:ollama", id: "anthropic/claude-sonnet-5", label: "Anthropic: Claude Sonnet 5", levels: ["off", "low", "medium", "high", "xhigh", "max"] },
  ]);
  expect(providerModels(catalog, "custom:gone")).toEqual([]);
  expect(providerModels(catalog, "grok")[0]?.label).toBe("Grok 4.7");
  expect(thinkingLevels(catalog, "custom:ollama", undefined).map((level) => level.id)).toEqual(["low", "medium", "high"]);
  expect(thinkingLevels(catalog, "custom:ollama", "anthropic/claude-sonnet-5").map((level) => level.id)).toEqual(["off", "low", "medium", "high", "xhigh", "max"]);
});

test("a custom provider's row has a server symbol and says Custom", () => {
  expect(providerSymbol("custom:ollama")).toBe("server.rack");
  expect(providerSubtitle("custom:ollama")).toBe("Custom");
  expect(usesAPIKey("custom:ollama")).toBe(false);
  expect([providerSymbol("anthropic"), providerSubtitle("anthropic"), usesAPIKey("anthropic")]).toEqual(["key.fill", "API key", true]);
  expect([providerSymbol("grok"), providerSubtitle("grok"), usesAPIKey("grok")]).toEqual(["person.badge.key.fill", "Subscription", false]);
});

test("the endpoint is the base URL cut back to its root, then the API's path", () => {
  expect(customEndpoint("messages", "https://api.anthropic.com/v1")).toBe("https://api.anthropic.com/v1/messages");
  expect(customEndpoint("messages", "https://api.anthropic.com/v1/messages")).toBe("https://api.anthropic.com/v1/messages");
  expect(customEndpoint("messages", "https://api.anthropic.com")).toBe("https://api.anthropic.com/v1/messages");
  expect(customEndpoint("chat-completions", "http://localhost:11434/v1/chat/completions")).toBe("http://localhost:11434/v1/chat/completions");
  expect(customEndpoint("chat-completions", " http://localhost:11434/v1// ")).toBe("http://localhost:11434/v1/chat/completions");
  expect(customEndpoint("responses", "https://x/v1/")).toBe("https://x/v1/responses");
  expect(customEndpoint("responses", "https://x/v1/responses/")).toBe("https://x/v1/responses");
  // Only the API's own path is cut, once.
  expect(customEndpoint("chat-completions", "https://x/v1/responses")).toBe("https://x/v1/responses/chat/completions");
  expect(customEndpoint("messages", "https://x/v1/v1")).toBe("https://x/v1/v1/messages");
});

test("the note under the base URL says what Lorca adds, then where requests go", () => {
  expect(customEndpointNote("chat-completions", "")).toBe("Lorca adds /chat/completions to it.");
  expect(customEndpointNote("messages", "   ")).toBe("Lorca adds /v1/messages to it.");
  expect(customEndpointNote("responses", "https://openrouter.ai/api/v1/")).toBe("Requests go to https://openrouter.ai/api/v1/responses.");
  expect(customBaseURLPlaceholder("messages")).toBe("https://api.example.com");
  expect(customBaseURLPlaceholder("responses")).toBe("https://api.example.com/v1");
});

test("the presets, in the Add Provider menu's order", () => {
  expect(customPresets.map((preset) => [preset.name, preset.api, preset.baseURL, preset.local])).toEqual([
    ["OpenAI", "responses", "https://api.openai.com/v1", false],
    ["OpenRouter", "chat-completions", "https://openrouter.ai/api/v1", false],
    ["Gemini", "chat-completions", "https://generativelanguage.googleapis.com/v1beta/openai", false],
    ["Groq", "chat-completions", "https://api.groq.com/openai/v1", false],
    ["Together AI", "chat-completions", "https://api.together.xyz/v1", false],
    ["Ollama", "chat-completions", "http://localhost:11434/v1", true],
    ["LM Studio", "chat-completions", "http://localhost:1234/v1", true],
  ]);
  expect(customPresets.map((preset) => preset.keyPlaceholder())).toEqual([
    "sk-… from platform.openai.com",
    "sk-or-… from openrouter.ai/keys",
    "Key from aistudio.google.com",
    "gsk_… from console.groq.com",
    "Key from api.together.ai",
    "Optional for a server on your network",
    "Optional for a server on your network",
  ]);
});

test("a preset the account has, by name in any case, is that provider", () => {
  const openRouter = customPresets.find((preset) => preset.name === "OpenRouter")!;
  const ollamaPreset = customPresets.find((preset) => preset.name === "Ollama")!;
  const mine: ProviderCredential = { ...ollama, kind: "custom:openrouter", name: "openrouter" };
  expect(presetProvider(openRouter, [ollama, mine])?.kind).toBe("custom:openrouter");
  expect(presetProvider(ollamaPreset, [ollama])?.kind).toBe("custom:ollama");
  expect(presetProvider(openRouter, [ollama])).toBeUndefined();
});

test("a base URL names its server by host and port, else its host", () => {
  expect(matchingPreset("http://localhost:11434/v1")?.name).toBe("Ollama");
  expect(matchingPreset(" http://localhost:1234 ")?.name).toBe("LM Studio");
  expect(matchingPreset("https://openrouter.ai/api/v1/chat/completions")?.name).toBe("OpenRouter");
  expect(matchingPreset("http://localhost:8080/v1")).toBeUndefined();
  expect(matchingPreset("api.openai.com")).toBeUndefined();
  expect(suggestedProviderName("http://localhost:11434/v1")).toBe("Ollama");
  expect(suggestedProviderName("https://llm.example.com:8443/v1")).toBe("llm.example.com");
  expect(suggestedProviderName("")).toBe("");
  expect(customHost("https://api.example.com/v1")).toBe("api.example.com");
  expect(customHost("not a url")).toBe("");
});

test("the sheet asks a server at an http or https URL with a host", () => {
  expect(isUsableBaseURL("http://localhost:11434/v1")).toBe(true);
  expect(isUsableBaseURL(" https://api.example.com ")).toBe(true);
  expect(isUsableBaseURL("http://")).toBe(false);
  expect(isUsableBaseURL("ftp://example.com")).toBe(false);
  expect(isUsableBaseURL("localhost:11434")).toBe(false);
  expect(isUsableBaseURL("")).toBe(false);
});

const model = (id: string, extra: Partial<CustomModel> = {}): CustomModel => ({ id, ...extra });
const ids = (checklist: ModelChecklist) => checklist.models.map((each) => each.id);
const picked = (checklist: ModelChecklist) => [...checklist.selected].sort();

test("a saved provider's models start picked, the first the default", () => {
  const saved = savedChecklist([model("b"), model("a")]);
  expect([ids(saved), picked(saved), saved.defaultID]).toEqual([["b", "a"], ["a", "b"], "b"]);
  const empty = savedChecklist();
  expect([ids(empty), empty.selected.size, empty.defaultID]).toEqual([[], 0, undefined]);
});

test("a listing keeps the picked and typed models where they are, and the rest of the old listing goes", () => {
  let checklist = takeListing(savedChecklist(), Array.from({ length: 9 }, (_, index) => model(`m${index}`)));
  // Nine models: too many to pick them all.
  expect([ids(checklist).length, checklist.selected.size, checklist.defaultID]).toEqual([9, 0, undefined]);
  checklist = toggleModel(checklist, "m4");
  checklist = addModel(checklist, "mine");
  expect(ids(checklist).slice(0, 2)).toEqual(["mine", "m0"]);
  const next = takeListing(checklist, [model("m4", { name: "Model Four", contextWindow: 128_000 }), model("n1"), model("n1"), model("mine", { images: true })]);
  expect(ids(next)).toEqual(["mine", "m4", "n1"]);
  expect(next.models[0]).toEqual({ id: "mine", images: true });
  expect(next.models[1]).toEqual({ id: "m4", name: "Model Four", contextWindow: 128_000 });
  expect([picked(next), next.defaultID]).toEqual([["m4", "mine"], "m4"]);
  // A server that lists nothing keeps the picked and typed models, none of the last server's.
  const unlisted = takeListing(toggleModel(next, "m4"), []);
  expect([ids(unlisted), picked(unlisted), unlisted.defaultID]).toEqual([["mine"], ["mine"], "mine"]);
});

test("a short listing starts picked when nothing is, and a saved pick keeps the rest unpicked", () => {
  const local = takeListing(savedChecklist(), [model("qwen3:8b"), model("gemma3:12b"), model("llava:13b")]);
  expect([picked(local), local.defaultID]).toEqual([["gemma3:12b", "llava:13b", "qwen3:8b"], "qwen3:8b"]);
  const edited = takeListing(savedChecklist([model("llava:13b")]), [model("qwen3:8b"), model("llava:13b", { images: true })]);
  expect([ids(edited), picked(edited), edited.defaultID]).toEqual([["llava:13b", "qwen3:8b"], ["llava:13b"], "llava:13b"]);
  expect(edited.models[0]?.images).toBe(true);
});

test("the default follows the picks", () => {
  let checklist = savedChecklist([model("a"), model("b"), model("c")]);
  checklist = toggleModel(checklist, "a");
  expect([picked(checklist), checklist.defaultID]).toEqual([["b", "c"], "b"]);
  // Picking the old default again leaves the new one.
  checklist = toggleModel(checklist, "a");
  expect(checklist.defaultID).toBe("b");
  checklist = toggleModel(toggleModel(toggleModel(checklist, "a"), "b"), "c");
  expect([checklist.selected.size, checklist.defaultID]).toEqual([0, undefined]);
  checklist = toggleModel(checklist, "c");
  expect(checklist.defaultID).toBe("c");
  // The first model added with nothing picked becomes the default.
  expect(addModel(savedChecklist(), "x").defaultID).toBe("x");
  expect(addModel(savedChecklist([model("a")]), "x").defaultID).toBe("a");
});

test("the search filters by name or id and offers to add an id no model has", () => {
  const models = [model("anthropic/claude-sonnet-5", { name: "Anthropic: Claude Sonnet 5" }), model("qwen3:8b"), model("Qwen3:32b")];
  expect(filterModels(models, "  ").map((each) => each.id)).toEqual(["anthropic/claude-sonnet-5", "qwen3:8b", "Qwen3:32b"]);
  expect(filterModels(models, "QWEN").map((each) => each.id)).toEqual(["qwen3:8b", "Qwen3:32b"]);
  expect(filterModels(models, "sonnet 5").map((each) => each.id)).toEqual(["anthropic/claude-sonnet-5"]);
  expect(addCandidate(" qwen3 ", models)).toBe("qwen3");
  expect(addCandidate("qwen3:8b", models)).toBeUndefined();
  expect(addCandidate("QWEN3:8B", models)).toBe("QWEN3:8B");
  expect(addCandidate("", models)).toBeUndefined();
  expect(modelDisplayName(models[0]!)).toBe("Anthropic: Claude Sonnet 5");
  expect(modelDisplayName(models[1]!)).toBe("qwen3:8b");
});

test("a provider saves its picked ids with the default first, then in list order", () => {
  let checklist = savedChecklist([model("a"), model("b"), model("c"), model("d")]);
  checklist = toggleModel(checklist, "b");
  expect(orderedModelIDs({ ...checklist, defaultID: "c" })).toEqual(["c", "a", "d"]);
  expect(orderedModelIDs(checklist)).toEqual(["a", "c", "d"]);
  expect(orderedModelIDs({ ...checklist, defaultID: undefined })).toEqual(["a", "c", "d"]);
});
