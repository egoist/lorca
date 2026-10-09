import { expect, test } from "bun:test";
import { exactAnswer, ExactObject, parseExact, stringifyExact } from "./exactJson";

const call = `{"owner":"lorca-app","issue_number":214,"channel_id":1234567890123456789,"ratio":0.000000000000000123,"tags":["a",true,null],"draft":{"2":"b","1":"a"}}`;

test("a call's arguments keep their numbers and key order through a parse, an edit, and back", () => {
  const args = parseExact(call);
  expect(stringifyExact(args)).toBe(call);
  const pretty = stringifyExact(args, true);
  expect(pretty).toContain('"channel_id": 1234567890123456789');
  expect(pretty.split("\n")[1]).toBe('  "owner": "lorca-app",');
  expect(stringifyExact(parseExact(pretty))).toBe(call);
  const edited = (args as ExactObject).with("owner", "someone");
  expect(stringifyExact(edited)).toBe(call.replace("lorca-app", "someone"));
  expect(stringifyExact((args as ExactObject).with("new", null))).toBe(call.slice(0, -1) + ',"new":null}');
});

test("text that isn't JSON is refused", () => {
  for (const bad of ["", "{", '{"a":1,}', "[1 2]", "{'a':1}", '{"a":01}', "nul", '{"a":1} x']) {
    expect(() => parseExact(bad)).toThrow(SyntaxError);
  }
  expect(parseExact(' "é\\n\\u00e9" ')).toBe("é\né");
});

test("the core's answer gives its result or throws its message", () => {
  expect(stringifyExact(exactAnswer(`{"result":{"id":9007199254740993}}`))).toBe(`{"id":9007199254740993}`);
  expect(() => exactAnswer(`{"error":{"message":"Workbench is offline."}}`)).toThrow("Workbench is offline.");
});
