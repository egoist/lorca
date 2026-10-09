// What the Limits screen and the rows that open it say, after the Mac's Budgets.swift: a limit
// set in a word or two, which limit work stopped at and what it used of it, and the form's
// fields. Kept free of React Native so `bun test` reads it.

import type { BudgetLimits, BudgetState, CallLimits } from "../core/model";
import { t, tc } from "../i18n";

export const isStopped = (b: BudgetState | undefined) => b?.state === "budget_exhausted" || b?.state === "interrupted";

/// "$5.00", "<$0.01".
export function dollars(value: number): string {
  return value > 0 && value < 0.01 ? "<$0.01" : `$${value.toFixed(2)}`;
}

/// Money the work used, an estimate marked as one: "$0.42", "$0.42 est.", or both.
export function spend(api: number, estimate: number): string {
  if (api > 0 && estimate > 0) return t("{api} + {estimate} est.", { api: dollars(api), estimate: dollars(estimate) });
  if (estimate > 0) return t("{amount} est.", { amount: dollars(estimate) });
  return dollars(api);
}

/// "12,400"
export const count = (value: number) => Math.round(value).toLocaleString("en-US");

/// "950", "12k", "1.2M"
function tokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${Math.floor(value / 1_000)}k`;
  return String(value);
}

/// "45 min", "2 h 30 min", "30 s".
export function minutes(seconds: number): string {
  if (seconds < 60) return t("{n} s", { n: Math.round(seconds) });
  const total = Math.round(seconds / 60);
  if (total < 60) return t("{n} min", { n: total });
  return total % 60 === 0 ? t("{n} h", { n: total / 60 }) : t("{h} h {m} min", { h: Math.floor(total / 60), m: total % 60 });
}

/// "$2.00 · 200k tokens", or None.
export function limitsSummary(limits: BudgetLimits | undefined): string {
  const parts: string[] = [];
  if (limits?.max_usd != null) parts.push(dollars(limits.max_usd));
  if (limits?.max_tokens != null) parts.push(t("{count} tokens", { count: tokens(limits.max_tokens) }));
  if (limits?.max_runtime_secs != null) parts.push(minutes(limits.max_runtime_secs));
  if (limits?.max_retries != null) parts.push(limits.max_retries === 1 ? t("1 retry") : t("{count} retries", { count: limits.max_retries }));
  if (limits?.max_connector_calls != null)
    parts.push(limits.max_connector_calls === 1 ? t("1 plugin call") : t("{count} plugin calls", { count: limits.max_connector_calls }));
  return parts.length ? parts.join(" · ") : tc("None", "limits");
}

/// The rows' word for a stopped allowance.
export const stoppedLabel = (b: BudgetState) => (b.state === "interrupted" ? t("Interrupted") : t("Limit reached"));

/// The stopped card's title: which limit, or the interruption.
export function stoppedTitle(b: BudgetState): string {
  if (b.state !== "budget_exhausted") return t("Interrupted");
  switch (b.reached) {
    case "usd":
      return t("Spending limit reached");
    case "tokens":
      return t("Token limit reached");
    case "runtime":
      return t("Run time limit reached");
    case "retries":
      return t("Retry limit reached");
    case "connector_calls":
      return t("Plugin call limit reached");
    case "unknown_price":
      return t("Price unknown");
  }
  return t("Limit reached");
}

/// Below the title: what it used of the limit it reached, or why it stopped.
export function stoppedDetail(b: BudgetState): string {
  if (b.state !== "budget_exhausted") return t("The Runner restarted while this was running. Check what it already did, then resume.");
  const { usage: u, limits: l } = b;
  switch (b.reached) {
    case "usd":
      return t("Used {used} of {limit}.", { used: spend(u.api_cost_usd, u.subscription_estimate_usd), limit: dollars(l.max_usd ?? 0) });
    case "tokens":
      return t("Used {used} of {limit} tokens.", { used: count(u.tokens), limit: count(l.max_tokens ?? 0) });
    case "runtime":
      return t("Ran {used} of {limit}.", { used: minutes(u.runtime_secs), limit: minutes(l.max_runtime_secs ?? 0) });
    case "retries":
      return t("Retried {used} of {limit} times.", { used: u.retries, limit: l.max_retries ?? 0 });
    case "connector_calls":
      return t("Made {used} of {limit} plugin calls.", { used: u.connector_calls, limit: l.max_connector_calls ?? 0 });
    case "unknown_price":
      return t("A spending limit can't cover a model without a known price. Add a token or run time limit.");
  }
  return t("Raise a limit to resume.");
}

/// New limits leave room for more work: the limit it reached went up (or away), and no other
/// limit is used up. Otherwise resuming needs the limits granted again in full.
export function fits(b: BudgetState, next: BudgetLimits): boolean {
  const raised = (n?: number | null, old?: number | null) => n == null || old == null || n > old;
  const room =
    b.reached === "usd" ? raised(next.max_usd, b.limits.max_usd)
    : b.reached === "unknown_price" ? next.max_usd == null || next.max_tokens != null || next.max_runtime_secs != null
    : b.reached === "tokens" ? raised(next.max_tokens, b.limits.max_tokens)
    : b.reached === "runtime" ? raised(next.max_runtime_secs, b.limits.max_runtime_secs)
    : b.reached === "retries" ? raised(next.max_retries, b.limits.max_retries)
    : b.reached === "connector_calls" ? raised(next.max_connector_calls, b.limits.max_connector_calls)
    : true;
  const u = b.usage;
  if (next.max_usd != null && u.api_cost_usd + u.subscription_estimate_usd >= next.max_usd) return false;
  if (next.max_tokens != null && u.tokens >= next.max_tokens) return false;
  if (next.max_runtime_secs != null && u.runtime_secs >= next.max_runtime_secs) return false;
  if (next.max_connector_calls != null && next.max_connector_calls > 0 && u.connector_calls >= next.max_connector_calls) return false;
  return room;
}

/// The form's fields as typed: dollars, tokens, minutes, retries, and plugin calls.
export interface LimitFields {
  usd: string;
  tokens: string;
  minutes: string;
  retries: string;
  calls: string;
}

export function fieldsOf(l: BudgetLimits | undefined): LimitFields {
  const whole = (v?: number | null) => (v == null ? "" : count(v));
  return {
    usd: l?.max_usd == null ? "" : l.max_usd.toFixed(2),
    tokens: whole(l?.max_tokens),
    minutes: l?.max_runtime_secs == null ? "" : String(Math.round(l.max_runtime_secs / 6) / 10),
    retries: whole(l?.max_retries),
    calls: whole(l?.max_connector_calls),
  };
}

/// The limits the fields hold, or the first field that isn't a number.
export function limitsOf(f: LimitFields): { limits: BudgetLimits } | { invalid: keyof LimitFields; message: string } {
  const limits: BudgetLimits = {};
  for (const key of ["usd", "tokens", "minutes", "retries", "calls"] as const) {
    const text = f[key].replace(/[,$\s]/g, "");
    if (!text) continue;
    const value = Number(text);
    if (!Number.isFinite(value) || value < 0 || ((key === "tokens" || key === "retries" || key === "calls") && !Number.isInteger(value)))
      return { invalid: key, message: t("Enter a number, or leave it empty for no limit.") };
    if (key === "minutes" && value > 525_600) return { invalid: key, message: t("Run time can be at most a year, or leave it empty for no limit.") };
    if (key === "usd") limits.max_usd = value;
    if (key === "tokens") limits.max_tokens = value;
    if (key === "minutes") limits.max_runtime_secs = Math.round(value * 60);
    if (key === "retries") limits.max_retries = value;
    if (key === "calls") limits.max_connector_calls = value;
  }
  return { limits };
}

/// "60 a minute", "10 every 30 s".
export function callSummary(l: CallLimits["limits"]): string {
  if (l.window_secs === 60) return t("{n} a minute", { n: l.max_calls });
  if (l.window_secs === 3600) return t("{n} an hour", { n: l.max_calls });
  if (l.window_secs === 86_400) return t("{n} a day", { n: l.max_calls });
  const every = l.window_secs % 3600 === 0 ? t("{n} h", { n: l.window_secs / 3600 }) : l.window_secs % 60 === 0 ? t("{n} min", { n: l.window_secs / 60 }) : t("{n} s", { n: l.window_secs });
  return t("{n} every {window}", { n: l.max_calls, window: every });
}
