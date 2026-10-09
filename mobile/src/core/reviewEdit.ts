// The `reviews.edit` request for what the user typed in a review's field, built from the item as
// the Runner holds it (exact JSON), so nothing the user did not touch changes: a command's other
// arguments, a call's server and tool, an id past 2^53. No React Native imports, so bun tests it.

import { ExactNumber, ExactObject, parseExact, stringifyExact, type Exact } from "./exactJson";

/// `reviews.edit`'s params as JSON text: the item's id and the version the user saw, and its
/// payload with `edited` in place of the draft's text, the command, or the call's arguments.
/// Throws a SyntaxError, or `notObject` as an Error, when a call's arguments aren't a JSON object.
export function reviewEditParams(item: Exact, edited: string, notObject: string): string {
  const fields = item instanceof ExactObject ? item : new ExactObject([]);
  const payload = fields.get("payload");
  const version = fields.get("version");
  if (!(payload instanceof ExactObject) || !(version instanceof ExactNumber)) throw new Error("No such review");
  let next: ExactObject;
  switch (payload.get("kind")) {
    case "draft":
      next = payload.with("text", edited);
      break;
    case "shell": {
      const args = payload.get("arguments");
      next = payload.with("arguments", (args instanceof ExactObject ? args : new ExactObject([])).with("command", edited));
      break;
    }
    default: {
      const args = parseExact(edited);
      if (!(args instanceof ExactObject)) throw new Error(notObject);
      next = payload.with("arguments", args);
    }
  }
  return stringifyExact(new ExactObject([["id", fields.get("id") ?? null], ["expected_version", version], ["payload", next]]));
}
