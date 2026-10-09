import { describe, expect, test } from "bun:test";
import type { BudgetState } from "../core/model";
import { callSummary, fieldsOf, fits, limitsOf, limitsSummary, spend, stoppedDetail, stoppedTitle } from "./limits";

const stopped: BudgetState = {
  kind: "job",
  id: "job-1",
  runner_id: "runner",
  chat_id: "chat",
  limits: { max_tokens: 100, max_runtime_secs: 600 },
  usage: { tokens: 100, api_cost_usd: 0.02, subscription_estimate_usd: 0, unknown_price_calls: 0, runtime_secs: 20, retries: 1, connector_calls: 2 },
  state: "budget_exhausted",
  reached: "tokens",
  updated_at: 1,
};

describe("limits", () => {
  test("a stopped turn says which limit, and resumes only when that limit went up", () => {
    expect(stoppedTitle(stopped)).toBe("Token limit reached");
    expect(stoppedDetail(stopped)).toBe("Used 100 of 100 tokens.");
    expect(fits(stopped, stopped.limits)).toBe(false);
    expect(fits(stopped, { max_tokens: 100, max_runtime_secs: 1200 })).toBe(false);
    expect(fits(stopped, { max_tokens: 200, max_runtime_secs: 600 })).toBe(true);
    expect(fits(stopped, { max_runtime_secs: 600 })).toBe(true);
    expect(fits(stopped, { max_tokens: 200, max_runtime_secs: 10 })).toBe(false);
  });

  test("fields keep zero apart from no limit and refuse what isn't a number", () => {
    expect(limitsOf({ usd: "0", tokens: "100,000", minutes: "15", retries: "", calls: "" })).toEqual({ limits: { max_usd: 0, max_tokens: 100000, max_runtime_secs: 900 } });
    expect(fieldsOf({ max_usd: 2, max_tokens: 200000 })).toEqual({ usd: "2.00", tokens: "200,000", minutes: "", retries: "", calls: "" });
    expect("invalid" in limitsOf({ usd: "", tokens: "1.5", minutes: "", retries: "", calls: "" })).toBe(true);
    expect("invalid" in limitsOf({ usd: "", tokens: "", minutes: "600000", retries: "", calls: "" })).toBe(true);
  });

  test("summaries", () => {
    expect(limitsSummary(undefined)).toBe("None");
    expect(limitsSummary({ max_usd: 2, max_tokens: 200000 })).toBe("$2.00 · 200k tokens");
    expect(spend(0.03, 0.4)).toBe("$0.03 + $0.40 est.");
    expect(callSummary({ max_calls: 60, window_secs: 60, max_concurrency: 4 })).toBe("60 a minute");
    expect(callSummary({ max_calls: 10, window_secs: 30, max_concurrency: 4 })).toBe("10 every 30 s");
  });
});
