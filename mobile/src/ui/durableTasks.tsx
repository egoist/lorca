// Durable tasks in the chat's details, after the desktop apps' Tasks section: a row per task
// with its state as a symbol, colored only where the task waits on the user. (The Running tasks
// of `app/tasks` are a chat's terminal commands, not these.)

import { Platform, PlatformColor, type ColorValue } from "react-native";
import type { DurableTaskState } from "../core/model";
import { t } from "../i18n";
import { accentColor, usePalette } from "./theme";

export function taskStateTitle(state: DurableTaskState): string {
  switch (state) {
    case "queued":
      return t("Not started");
    case "working":
      return t("Working");
    case "blocked":
      return t("Blocked");
    case "awaiting_review":
      return t("Ready for review");
    case "completed":
      return t("Completed");
    case "cancelled":
      return t("Cancelled");
  }
}

/// The state's color: orange for a blocked task, the tint for one ready for review, quiet
/// otherwise.
export function useTaskTint(): (state: DurableTaskState) => ColorValue {
  const p = usePalette();
  return (state) => {
    switch (state) {
      case "blocked":
        return Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
      case "awaiting_review":
        return p.tint;
      case "completed":
      case "cancelled":
        return p.tertiaryLabel;
      default:
        return p.secondaryLabel;
    }
  };
}
