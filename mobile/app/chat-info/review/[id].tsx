// One thing a bot left for review, slid in from the chat's details, after the desktop apps' review
// sheet and the permission card it would have asked with: who wants to do what and why, then the
// command, call, or draft in one field to edit, and the one decision. Approve, the toolbar's
// button, runs what the field shows on the bot's Runner, saving an edit first as a new version;
// Reject says no. Once decided, the screen shows how it went.
//
// A change from another Device shows as it lands, unless the user is editing; then Approve offers
// the new version instead of deciding on the old one. A call's arguments come from the core as
// its exact JSON text, so an id past 2^53 shows and goes back as it was.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { ExactObject, stringifyExact } from "../../../src/core/exactJson";
import type { ReviewItem } from "../../../src/core/model";
import { useBotMap, useReview, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";
import { reviewStateWord, reviewSymbol, reviewTitle } from "../../../src/ui/reviews";
import { Symbol } from "../../../src/ui/Symbol";
import { accentColor, usePalette } from "../../../src/ui/theme";

/// What the field shows for `item`: a draft's text or a command; a call's arguments come exact
/// from the core instead.
const plainText = (item: ReviewItem) => (item.payload.kind === "draft" ? item.payload.text : item.payload.kind === "shell" ? (item.payload.arguments.command ?? "") : undefined);

export default function ReviewScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const bots = useBotMap();
  const devices = useStore((s) => s.devices);
  const latest = useReview(id);
  // The version on screen, the field's text for it (undefined while a call's arguments load), and
  // what the field holds now.
  const [shown, setShown] = useState(latest);
  const [base, setBase] = useState(latest && plainText(latest));
  const [text, setText] = useState(base ?? "");
  const [busy, setBusy] = useState(false);
  const edited = base !== undefined && text !== base;

  function show(item: ReviewItem, keepingEdits: boolean) {
    setShown(item);
    const plain = plainText(item);
    if (plain !== undefined) {
      setBase(plain);
      if (!keepingEdits) setText(plain);
      return;
    }
    if (keepingEdits) return;
    setBase(undefined);
    engine
      .exactReview(item.id)
      .then((exact) => {
        const payload = exact.get("payload");
        const pretty = stringifyExact((payload instanceof ExactObject ? payload.get("arguments") : undefined) ?? null, true);
        setBase(pretty);
        setText(pretty);
      })
      .catch((error) => alert(t("Couldn't load this"), error instanceof Error ? error.message : String(error)));
  }

  useEffect(() => {
    if (latest) show(latest, false);
  }, []);

  // The item as another Device or the Runner left it.
  useEffect(() => {
    if (!latest || !shown || busy || latest.revision === shown.revision) return;
    const sameVersion = latest.state === "pending" && latest.version === shown.version;
    if (sameVersion || !(latest.state === "pending" && edited)) show(latest, sameVersion);
  }, [latest?.revision]);

  if (!shown) return null;
  const pending = shown.state === "pending";
  const bot = bots.get(shown.bot_id);
  const runner = devices.find((device) => device.id === shown.runner_id);
  const state = reviewStateWord(shown);
  const output = shown.outcome?.result?.text || undefined;
  // Why it needs another look, or why it may have run without a result.
  const note = shown.outcome && (pending || shown.state === "uncertain") ? shown.outcome.summary : undefined;
  const mono = shown.payload.kind !== "draft";

  async function decide(failure: string, request: () => Promise<unknown>) {
    setBusy(true);
    try {
      await request();
      router.back();
    } catch (error) {
      setBusy(false);
      const now = useStore.getState().reviews.find((item) => item.id === shown!.id);
      if (now && now.revision !== shown!.revision) show(now, now.state === "pending" && now.version === shown!.version);
      alert(failure, error instanceof Error ? error.message : String(error));
    }
  }

  function approve() {
    if (busy || !shown || base === undefined) return;
    // Edited here while it changed elsewhere: approving would mean the wrong text.
    if (latest && latest.version !== shown.version && edited) {
      alert(t("This changed on another Device"), t("Show Latest discards your changes and shows the current version to review."), [
        { text: t("Cancel"), style: "cancel" },
        { text: t("Show Latest"), onPress: () => show(latest, false) },
      ]);
      return;
    }
    void decide(t("Couldn't approve"), () => engine.approveReview(shown, edited ? text : undefined));
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Review") }} />
      {pending ? <SaveToolbar label={t("Approve")} disabled={busy || base === undefined} onSave={approve} /> : null}
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <View style={styles.header}>
          <Symbol name={reviewSymbol(shown, runner)} size={22} color={p.secondaryLabel} />
          <View style={styles.headerText}>
            <Text style={[styles.title, { color: p.label }]}>{reviewTitle(shown, bot, runner)}</Text>
            {shown.rationale ? <Text style={[styles.rationale, { color: p.secondaryLabel }]}>{shown.rationale}</Text> : null}
          </View>
        </View>

        <Section footer={pending ? note : undefined}>
          <FieldRow
            value={text}
            onChangeText={setText}
            multiline
            editable={pending && !busy && base !== undefined}
            accessibilityLabel={shown.payload.kind === "draft" ? t("Draft") : shown.payload.kind === "shell" ? t("Command") : t("Arguments")}
            autoCapitalize={mono ? "none" : "sentences"}
            autoCorrect={!mono}
            spellCheck={!mono}
            style={[mono ? styles.mono : null, shown.payload.kind === "shell" ? styles.command : styles.editor]}
          />
        </Section>

        <Section title={t("Details")} footer={pending ? undefined : note}>
          {state ? (
            <Row
              title={t("Status")}
              accessory={
                <Text style={[styles.state, { color: shown.state === "uncertain" || shown.state === "failed" ? accentColor("orange", p.dark) : p.secondaryLabel }]}>{state}</Text>
              }
            />
          ) : null}
          <Row title={t("Account")} detail={shown.target.account} />
          {/* A short value on the right, as Account's; a call's summary under the label. */}
          {shown.target.resource.length > 28 ? <Row title={t("Resource")} subtitle={shown.target.resource} subtitleLines={4} /> : <Row title={t("Resource")} detail={shown.target.resource} />}
          {shown.preconditions.files.length > 0 ? <Row title={t("Files")} subtitle={shown.preconditions.files.map((file) => file.path).join("\n")} subtitleLines={16} /> : null}
        </Section>

        {output ? (
          <Section title={t("Output")}>
            <View style={styles.output}>
              <Text selectable style={[styles.mono, { color: p.label }]}>
                {output}
              </Text>
            </View>
          </Section>
        ) : null}

        {pending ? (
          <Section>
            <Row title={t("Reject")} icon="xmark.circle" destructive onPress={busy ? undefined : () => void decide(t("Couldn't reject"), () => engine.rejectReview(shown))} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", gap: 12, paddingHorizontal: 20, paddingTop: 8, paddingBottom: 16 },
  headerText: { flex: 1, gap: 4 },
  title: { fontSize: 17, fontWeight: "600", lineHeight: 22 },
  rationale: { fontSize: 15, lineHeight: 20 },
  mono: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 13, lineHeight: 18 },
  command: { minHeight: 44 },
  editor: { minHeight: 200 },
  output: { paddingVertical: 11, paddingHorizontal: 16 },
  state: { fontSize: 17 },
});
