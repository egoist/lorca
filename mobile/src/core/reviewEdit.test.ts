import { expect, test } from "bun:test";
import { parseExact } from "./exactJson";
import { reviewEditParams } from "./reviewEdit";

const item = (payload: string) => parseExact(`{"id":"review-1","version":3,"revision":7,"payload":${payload}}`);

test("a command's edit keeps the other arguments", () => {
  const params = reviewEditParams(item(`{"kind":"shell","arguments":{"command":"git push","description":"Push","timeout_ms":120000}}`), "git push --tags", "");
  expect(params).toBe(`{"id":"review-1","expected_version":3,"payload":{"kind":"shell","arguments":{"command":"git push --tags","description":"Push","timeout_ms":120000}}}`);
});

test("a call's edit keeps its server, tool, and exact numbers", () => {
  const call = `{"kind":"plugin","plugin_id":"slack-0a1b","server_name":"main","tool":"post","arguments":{"channel":1234567890123456789}}`;
  const params = reviewEditParams(item(call), `{\n  "channel": 1234567890123456789,\n  "text": "Shipped"\n}`, "");
  expect(params).toBe(`{"id":"review-1","expected_version":3,"payload":{"kind":"plugin","plugin_id":"slack-0a1b","server_name":"main","tool":"post","arguments":{"channel":1234567890123456789,"text":"Shipped"}}}`);
  expect(() => reviewEditParams(item(call), "[1, 2]", "needs an object")).toThrow("needs an object");
  expect(() => reviewEditParams(item(call), "{oops", "needs an object")).toThrow(SyntaxError);
});

test("a draft's edit replaces its text", () => {
  expect(reviewEditParams(item(`{"kind":"draft","text":"Hi"}`), "Hi team\n\nShipped", "")).toBe(`{"id":"review-1","expected_version":3,"payload":{"kind":"draft","text":"Hi team\\n\\nShipped"}}`);
});
