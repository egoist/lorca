import { expect, mock, test } from "bun:test";
import type { Body, SavedSecret } from "../core/model";

mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
mock.module("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
const { secretCaption, secretPlace, secretTitle } = await import("./secrets");

type Card = Extract<Body, { kind: "permission" }>;
const card = (use: "browser" | "command" | "plugin", decision: Card["decision"] = "pending"): Card => ({
  kind: "permission", plugin_id: use === "plugin" ? "acme" : "playwright", plugin_name: use === "plugin" ? "Acme" : "Browser", tool: "secret",
  summary: "GitHub password, One-time code", decision, reason: "To open the pull request.",
  secret: { use, site: use === "browser" ? "github.com" : undefined, fields: [{ name: "password", label: "GitHub password" }, { name: "otp", label: "One-time code" }] },
});

test("the card says who asks and where it goes", () => {
  expect(secretTitle(card("browser"), "Developer")).toBe("Developer needs a secret for github.com");
  expect(secretTitle(card("plugin"), "Developer")).toBe("Developer needs a secret for Acme");
  expect(secretTitle(card("command"), "Developer")).toBe("Developer needs a secret for its commands");
});

test("why while it asks; the answer once answered", () => {
  expect(secretCaption(card("browser"))).toBe("To open the pull request.");
  expect(secretCaption(card("browser", "allowed"))).toBe("Saved · GitHub password, One-time code");
  expect(secretCaption(card("browser", "denied"))).toBe("Not now · GitHub password, One-time code");
});

test("a saved secret says where it goes", () => {
  const secret = (use: SavedSecret["use"], site?: string): SavedSecret => ({ id: "s", bot_id: "b", name: "n", label: "l", use, site, updated_at: 1 });
  expect(secretPlace(secret("browser", "github.com"))).toBe("github.com");
  expect(secretPlace(secret("command"))).toBe("Commands");
});
