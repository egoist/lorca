import { expect, mock, test } from "bun:test";

const sent: { method: string; params: unknown }[] = [];
let failNext = false;
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
mock.module("../../modules/lorca-core", () => ({
  request: async (method: string, params: unknown) => {
    sent.push({ method, params });
    if (failNext) {
      failNext = false;
      throw new Error("The Runner did not answer.");
    }
    if (method === "bots.permissions") return { connections: [{ id: "github", name: "GitHub", tools: [{ name: "search_issues", capability: "read" }] }] };
    return {};
  },
}));
const access = await import("./access");
const { useStore } = await import("./store");

const github = {
  id: "github",
  name: "GitHub",
  tools: [
    { name: "search_issues", capability: "read" },
    { name: "create_pull_request_review", capability: "draft" },
    { name: "create_issue", capability: "write" },
  ],
};
const linear = { id: "linear", name: "Linear", tools: [] };
const plugins = [github, linear];

test("no policy is full access, and levels take in the ones below them", () => {
  expect(access.isFullAccess(undefined)).toBe(true);
  expect(access.accessSummary(undefined)).toBe("Full access");
  expect(access.pluginLevel(undefined, "github")).toBe("write");
  expect(access.levelOf(["read", "draft"])).toBe("draft");
  expect(access.reaches("draft", "write")).toBe(false);
  expect(access.reaches("draft", "read")).toBe(true);
  expect(access.accessSummary({ shell: false })).toBe("Limited");
});

test("narrowing one plugin lists every plugin, and opening it all again lists none", () => {
  const narrowed = access.withPlugin(undefined, plugins, "github", "draft", undefined);
  expect(narrowed.connections).toEqual({ github: { capabilities: ["read", "draft"] }, linear: { capabilities: ["read", "draft", "write"] } });
  const off = access.withPlugin(narrowed, plugins, "linear", "none", undefined);
  expect(off.connections).toEqual({ github: { capabilities: ["read", "draft"] } });
  expect(access.pluginLevel(off, "linear")).toBe("none");
  const open = access.withPlugin(access.withPlugin(off, plugins, "linear", "write", undefined), plugins, "github", "write", undefined);
  expect(open.connections).toBeUndefined();
  expect(open).toEqual({ filesystem: "write", shell: true });
});

test("tools are chosen one by one, and choosing every tool is all of them", () => {
  const without = access.withTool(undefined, plugins, github, "create_issue", false);
  expect(without.connections?.github).toEqual({ capabilities: ["read", "draft", "write"], tools: ["create_pull_request_review", "search_issues"] });
  expect(access.toolsSummary(without, github)).toBe("2 of 3 tools");
  const back = access.withTool(without, plugins, github, "create_issue", true);
  expect(back.connections).toBeUndefined();
  expect(access.toolsSummary(back, github)).toBe("All tools");
  expect(access.toolsSummary(back, linear)).toBeUndefined();
});

test("files and shell keep the plugins as they are", () => {
  const policy = access.withPlugin(undefined, plugins, "github", "read", undefined);
  expect(access.filesLevel({ ...policy, filesystem: "none" })).toBe("none");
  expect(access.shellOn({ ...policy, shell: false })).toBe(false);
  expect(access.pluginLevel({ ...policy, shell: false }, "github")).toBe("read");
});

test("a save shows at once, goes to the core, and a failed one puts the old Access back", async () => {
  useStore.setState({ bots: [{ id: "b", name: "Writer", description: "", symbol_name: "pencil", accent: "pink", runner_id: "r", provider: "deepseek", created_at: 0 }] });
  await access.setBotPermissions("b", { shell: false });
  expect(useStore.getState().bots[0].permissions).toEqual({ shell: false });
  expect(sent.at(-1)).toEqual({ method: "bots.update", params: { id: "b", permissions: { shell: false } } });
  failNext = true;
  await expect(access.setBotPermissions("b", { filesystem: "none" })).rejects.toThrow("did not answer");
  expect(useStore.getState().bots[0].permissions).toEqual({ shell: false });
  await access.loadAccessCatalog("b");
  expect(access.useAccessStore.getState().byBot.b).toEqual([{ id: "github", name: "GitHub", tools: [{ name: "search_issues", capability: "read" }] }]);
  failNext = true;
  await access.loadAccessCatalog("b");
  expect(Array.isArray(access.useAccessStore.getState().byBot.b)).toBe(true);
});
