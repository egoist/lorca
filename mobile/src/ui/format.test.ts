import { expect, setSystemTime, test } from "bun:test";
import { ago } from "./format";

test("ago counts in the largest unit that fits", () => {
  setSystemTime(new Date(1_000_000 * 1000));
  expect(ago(1_000_000 - 5)).toBe("just now");
  expect(ago(1_000_000 - 300)).toBe("5m ago");
  expect(ago(1_000_000 - 3 * 3600)).toBe("3h ago");
  expect(ago(1_000_000 - 2 * 86400)).toBe("2d ago");
  setSystemTime();
});
