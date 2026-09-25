// What the phone says when it cannot connect to its relay: a few words where the relay status
// shows, and on a tap the error itself with what to do about it.

import { Alert } from "react-native";
import { engine } from "../core/engine";
import type { RelayProblem } from "../core/model";
import { t } from "../i18n";

/// The few words for the chat list's title slot.
export function problemTitle(problem: RelayProblem): string {
  return problem.unknown_machine ? t("Pair this phone again") : t("Can’t connect");
}

/// The error as the core has it and what it means. A relay with no record of this phone takes
/// it back only through pairing again, so that one offers Unpair; anything else the core keeps
/// trying, and Try Now asks again without waiting out the backoff.
export function showRelayProblem(problem: RelayProblem, url: string | null) {
  const relay = url?.replace(/^https?:\/\//, "") ?? "—";
  if (problem.unknown_machine) {
    Alert.alert(
      t("Pair this phone again"),
      t("The relay at {relay} has no record of this phone (“{error}”), so it was probably reset. Unpair this phone, then pair it again with a new code from your computer, which keeps your chats.", { relay, error: problem.message }),
      [
        { text: t("Cancel"), style: "cancel" },
        // Forgetting the identity flips `paired`, and the guarded stack swaps to the pair screen.
        { text: t("Unpair This Phone"), style: "destructive", onPress: () => void engine.unpair() },
      ],
    );
    return;
  }
  Alert.alert(t("Can’t connect to the relay"), t("The last try to reach the relay at {relay} failed:\n{error}\n\nLorca keeps trying.", { relay, error: problem.message }), [
    { text: t("OK"), style: "cancel" },
    { text: t("Try Now"), onPress: () => engine.notify() },
  ]);
}
