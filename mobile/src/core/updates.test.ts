import { beforeEach, expect, mock, test } from "bun:test";

// The Android app's check for a newer release, against a GitHub API answered here: tags, drafts
// and prerelease releases, the APK and its digest, and the daily check's alert.
let prefs: Record<string, unknown> = {};
const alerts: { title: string; buttons: { text: string; onPress?: () => void }[] }[] = [];
const installs: unknown[][] = [];
mock.module("react-native", () => ({
  Platform: { OS: "android" },
  Alert: { alert: (title: string, _message: string, buttons: (typeof alerts)[number]["buttons"]) => alerts.push({ title, buttons }) },
  AppState: { addEventListener: () => {} },
}));
mock.module("expo-application", () => ({ applicationId: "app.lorca", nativeApplicationVersion: "1.2.0" }));
mock.module("./prefs", () => ({ loadPrefs: () => prefs, savePrefs: (next: Record<string, unknown>) => { prefs = next; } }));
mock.module("../../modules/lorca-core", () => ({
  installUpdate: async (...args: unknown[]) => { installs.push(args); },
  onInstallStatus: () => () => {},
}));

/// What GitHub answers, by path under the repository.
let routes: Record<string, unknown> = {};
globalThis.fetch = (async (input: string | URL) => {
  const path = String(input).replace("https://api.github.com/repos/egoist/lorca", "");
  const body = routes[path];
  if (body === undefined) return new Response("{}", { status: 404 });
  return Response.json(body);
}) as typeof fetch;

const updates = await import("./updates");

function release(version: string, extra: Record<string, unknown> = {}) {
  return {
    draft: false,
    prerelease: false,
    body: `Notes for ${version}`,
    assets: [{ name: `lorca-${version}-android.apk`, browser_download_url: `https://example.test/${version}.apk`, size: 42, digest: `sha256:${"ab".repeat(32)}` }],
    ...extra,
  };
}

function tags(...versions: string[]) {
  return versions.map((v) => ({ ref: `refs/tags/mobile-v${v}` }));
}

beforeEach(() => {
  prefs = {};
  alerts.length = 0;
  installs.length = 0;
  routes = {};
  updates.useUpdates.setState({ phase: "idle", latest: undefined, error: undefined, checked: false, progress: 0 });
});

test("versions compare by number", () => {
  expect(updates.isNewer("1.10.0", "1.9.9")).toBe(true);
  expect(updates.isNewer("2.0.0", "1.99.99")).toBe(true);
  expect(updates.isNewer("1.2.0", "1.2.0")).toBe(false);
  expect(updates.isNewer("1.1.9", "1.2.0")).toBe(false);
});

test("the newest published release newer than the build, past drafts and prereleases", async () => {
  routes = {
    "/git/matching-refs/tags/mobile-v": tags("1.1.0", "1.2.0", "1.3.0", "1.4.0", "1.5.0", "1.10.0"),
    // 1.10.0 is still a draft, which answers 404 to an unauthenticated read.
    "/releases/tags/mobile-v1.5.0": release("1.5.0", { prerelease: true }),
    "/releases/tags/mobile-v1.4.0": release("1.4.0", { assets: [] }),
    "/releases/tags/mobile-v1.3.0": release("1.3.0"),
    "/releases/tags/mobile-v1.1.0": release("1.1.0"),
  };
  expect(await updates.newestRelease()).toEqual({
    version: "1.3.0",
    notes: "Notes for 1.3.0",
    url: "https://example.test/1.3.0.apk",
    sha256: "ab".repeat(32),
    size: 42,
  });
});

test("nothing newer than the build is up to date", async () => {
  routes = { "/git/matching-refs/tags/mobile-v": tags("1.1.0", "1.2.0"), "/releases/tags/mobile-v1.2.0": release("1.2.0") };
  expect(await updates.newestRelease()).toBeNull();
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", latest: undefined, checked: true });
});

test("an APK without a digest is not offered", async () => {
  routes = {
    "/git/matching-refs/tags/mobile-v": tags("1.3.0"),
    "/releases/tags/mobile-v1.3.0": release("1.3.0", { assets: [{ name: "lorca-1.3.0-android.apk", browser_download_url: "x", size: 1, digest: null }] }),
  };
  expect(await updates.newestRelease()).toBeNull();
});

test("the daily check offers a release once, and Skip This Version keeps it from asking again", async () => {
  routes = { "/git/matching-refs/tags/mobile-v": tags("1.3.0"), "/releases/tags/mobile-v1.3.0": release("1.3.0") };
  await updates.checkForUpdates(true);
  expect(updates.useUpdates.getState().latest?.version).toBe("1.3.0");
  expect(prefs.update_checked_at).toBeNumber();
  expect(alerts.map((a) => a.title)).toEqual(["Lorca 1.3.0 is available"]);
  expect(alerts[0]!.buttons.map((b) => b.text)).toEqual(["Skip This Version", "Remind Me Later", "Install Update"]);

  alerts[0]!.buttons[0]!.onPress!();
  expect(prefs.update_skipped).toBe("1.3.0");
  await updates.checkForUpdates(true);
  expect(alerts).toHaveLength(1);
  // A check of the user's own still finds it, for Settings to install.
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState().latest?.version).toBe("1.3.0");
});

test("Install Update hands the APK, its digest, and its size to the native module", async () => {
  routes = { "/git/matching-refs/tags/mobile-v": tags("1.3.0"), "/releases/tags/mobile-v1.3.0": release("1.3.0") };
  await updates.checkForUpdates(true);
  alerts[0]!.buttons[2]!.onPress!();
  await Promise.resolve();
  expect(installs).toEqual([["https://example.test/1.3.0.apk", "ab".repeat(32), 42]]);
  expect(updates.useUpdates.getState().phase).toBe("downloading");
});

test("a failed check keeps its reason for Settings", async () => {
  globalThis.fetch = (async () => new Response("{}", { status: 403 })) as unknown as typeof fetch;
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", error: "GitHub answered 403", checked: true });
});
