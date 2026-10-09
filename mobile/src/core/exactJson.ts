// JSON whose numbers keep their text and whose objects keep their key order, for a review's plugin
// call: `JSON.parse` rounds an integer past 2^53 (a Discord or Twitter id), and the phone must
// show and send back exactly what the Runner holds. No React Native imports, so bun tests it.

/// A number as the JSON spelled it.
export class ExactNumber {
  constructor(readonly text: string) {}
}

/// An object as an ordered list of its members.
export class ExactObject {
  constructor(readonly members: [string, Exact][]) {}

  get(key: string): Exact | undefined {
    return this.members.find(([name]) => name === key)?.[1];
  }

  with(key: string, value: Exact): ExactObject {
    const at = this.members.findIndex(([name]) => name === key);
    return new ExactObject(at < 0 ? [...this.members, [key, value]] : this.members.map((member, index) => (index === at ? [key, value] : member)));
  }
}

export type Exact = null | boolean | string | ExactNumber | Exact[] | ExactObject;

const NUMBER = /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/y;
const STRING = /"(?:[^"\\\u0000-\u001f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"/y;

/// Parses `text`, throwing a SyntaxError where it is not JSON.
export function parseExact(text: string): Exact {
  let at = 0;
  const space = () => {
    while (at < text.length && " \t\n\r".includes(text[at])) at++;
  };
  const fail = (): never => {
    throw new SyntaxError(`Unexpected ${at < text.length ? `“${text[at]}”` : "end"} at ${at}`);
  };
  const token = (pattern: RegExp) => {
    pattern.lastIndex = at;
    const match = pattern.exec(text);
    if (!match) fail();
    at = pattern.lastIndex;
    return match![0];
  };
  const value = (): Exact => {
    space();
    const char = text[at];
    if (char === "{") {
      at++;
      const members: [string, Exact][] = [];
      space();
      if (text[at] === "}") return at++, new ExactObject(members);
      for (;;) {
        space();
        const key = JSON.parse(token(STRING)) as string;
        space();
        if (text[at++] !== ":") at--, fail();
        members.push([key, value()]);
        space();
        if (text[at] === ",") at++;
        else if (text[at] === "}") return at++, new ExactObject(members);
        else fail();
      }
    }
    if (char === "[") {
      at++;
      const items: Exact[] = [];
      space();
      if (text[at] === "]") return at++, items;
      for (;;) {
        items.push(value());
        space();
        if (text[at] === ",") at++;
        else if (text[at] === "]") return at++, items;
        else fail();
      }
    }
    if (char === '"') return JSON.parse(token(STRING)) as string;
    for (const [word, literal] of [["true", true], ["false", false], ["null", null]] as const) {
      if (text.startsWith(word, at)) return (at += word.length), literal;
    }
    return new ExactNumber(token(NUMBER));
  };
  const result = value();
  space();
  if (at < text.length) fail();
  return result;
}

/// `value` as JSON, indented two spaces a level when `pretty`.
export function stringifyExact(value: Exact, pretty = false, indent = ""): string {
  const inner = pretty ? indent + "  " : "";
  const open = pretty ? "\n" + inner : "";
  const close = pretty ? "\n" + indent : "";
  const comma = pretty ? ",\n" + inner : ",";
  if (value === null || typeof value === "boolean") return String(value);
  if (typeof value === "string") return JSON.stringify(value);
  if (value instanceof ExactNumber) return value.text;
  if (Array.isArray(value)) return value.length ? "[" + open + value.map((item) => stringifyExact(item, pretty, inner)).join(comma) + close + "]" : "[]";
  if (!value.members.length) return "{}";
  return "{" + open + value.members.map(([key, item]) => JSON.stringify(key) + (pretty ? ": " : ":") + stringifyExact(item, pretty, inner)).join(comma) + close + "}";
}

/// The core's answer (`{ result }` or `{ error: { message } }`) to a request, as exact JSON;
/// throws with the core's message for an error.
export function exactAnswer(raw: string): Exact {
  const answer = parseExact(raw);
  const error = answer instanceof ExactObject ? answer.get("error") : undefined;
  if (error instanceof ExactObject) throw new Error(String(error.get("message") ?? "The request failed."));
  return answer instanceof ExactObject ? (answer.get("result") ?? null) : null;
}
