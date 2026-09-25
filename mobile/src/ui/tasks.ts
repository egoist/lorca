// What the Running tasks sheet reads, after the Mac's popover: the time moving on while it shows,
// and where a command stands in words.

import { useEffect, useState } from "react";
import type { CommandRun } from "../core/model";
import { t } from "../i18n";

/// The time now, moving on every second while the caller is on screen.
export function useNow(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

/// "0:42", "12:03", "1:02:03".
export function elapsed(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const [hours, minutes, rest] = [Math.floor(whole / 3600), Math.floor((whole % 3600) / 60), whole % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(rest)}` : `${minutes}:${pad(rest)}`;
}

/// "Running · 1:05" and "Waiting for input · 1:05" while it runs, counted from its row's start
/// (unix seconds); how it ended once it has.
export function taskState(run: CommandRun, startedAt: number, now: number): string {
  const running = elapsed(now / 1000 - startedAt);
  switch (run.state) {
    case "running":
      return `${t("Running")} · ${running}`;
    case "waiting":
      return `${t("Waiting for input")} · ${running}`;
    case "exited":
      return t("Finished");
    case "failed":
      return t("Failed");
    default:
      return t("Stopped");
  }
}
