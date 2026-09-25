// A running task, as the chat's Running tasks sheet lists it: a command a bot has running in its
// terminal, after the Mac's popover. What it does in the bot's words with Stop on that line, who
// runs it and for how long, the command on one line (a tap shows all of it), and its last lines.
// One that ends while the sheet is open says how it ended instead of Stop.

import { useEffect, useState } from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { isLive, type Bot, type Message } from "../core/model";
import { t, useLanguage } from "../i18n";
import { firstLine } from "./format";
import { Symbol } from "./Symbol";
import { CommandSheet, OutputBlock } from "./transcript";
import { usePalette } from "./theme";

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

export function TaskCard({ message, bot, isGroup, now, onStop }: { message: Message; bot: Bot | undefined; isGroup: boolean; now: number; onStop: () => Promise<void> }) {
  useLanguage();
  const p = usePalette();
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showCommand, setShowCommand] = useState(false);
  const body = message.body.kind === "tool" ? message.body : undefined;
  const run = body?.run;
  if (!body || !run) return null;
  const live = isLive(run);
  const who = bot?.name ?? t("The bot");
  const running = elapsed(now / 1000 - message.created_at);
  const state = run.state === "running" ? `${t("Running")} · ${running}` : run.state === "waiting" ? `${t("Waiting for input")} · ${running}` : run.state === "exited" ? t("Finished") : run.state === "failed" ? t("Failed") : t("Stopped");
  const stop = async () => {
    setStopping(true);
    setError(null);
    try {
      await onStop();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setStopping(false);
    }
  };
  return (
    <View style={[styles.card, { backgroundColor: p.cell, borderColor: p.separator }]}>
      <View style={styles.titleLine}>
        <Symbol name="terminal" size={16} color={p.tint} />
        <Text style={[styles.title, { color: p.label }]} numberOfLines={2}>
          {body.description ?? firstLine(run.command)}
        </Text>
        {live ? (
          <Pressable
            disabled={stopping}
            onPress={() => void stop()}
            hitSlop={6}
            style={({ pressed }) => [styles.stop, { backgroundColor: pressed ? p.separator : p.fill, opacity: stopping ? 0.5 : 1 }]}
            accessibilityRole="button"
          >
            <Text style={{ color: p.label, fontSize: 13, fontWeight: "600" }}>{t("Stop")}</Text>
          </Pressable>
        ) : null}
      </View>
      <Text style={[styles.status, { color: p.secondaryLabel }]} numberOfLines={1}>
        {isGroup ? `${who} · ${state}` : state}
      </Text>
      <Pressable
        onPress={() => setShowCommand(true)}
        style={({ pressed }) => [styles.command, { backgroundColor: p.code, opacity: pressed ? 0.6 : 1 }]}
        accessibilityRole="button"
        accessibilityLabel={t("Show the full command")}
      >
        <Text style={[styles.commandText, { color: p.label }]} numberOfLines={1}>
          {`$ ${firstLine(run.command)}`}
        </Text>
      </Pressable>
      {run.output ? <OutputBlock text={run.output.split("\n").filter(Boolean).join("\n")} lines={10} /> : null}
      {error && live ? <Text style={[styles.status, { color: p.red }]}>{error}</Text> : null}
      <CommandSheet visible={showCommand} title={t("{who}'s command", { who })} command={run.command} onClose={() => setShowCommand(false)} />
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderRadius: 14, borderWidth: StyleSheet.hairlineWidth, paddingHorizontal: 12, paddingVertical: 10, gap: 6 },
  titleLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  title: { fontSize: 14, fontWeight: "600", flex: 1 },
  stop: { paddingHorizontal: 10, paddingVertical: 4, borderRadius: 7 },
  status: { fontSize: 13, lineHeight: 18 },
  command: { borderRadius: 8, paddingHorizontal: 10, paddingVertical: 7, marginTop: 2 },
  commandText: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12.5, lineHeight: 17 },
});
