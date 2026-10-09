// One output, slid in inside the chat's details, after the macOS app's output sheet: a preview of
// the file or the link, what the bot checked, and its versions, any of which the screen can
// show. Open shows the file in Quick Look (iOS) or the link in the browser; Share hands either to
// the share sheet.

import { Stack, useLocalSearchParams } from "expo-router";
import { useState } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { documentUrl, evidenceStatus, evidenceTitle } from "../../../src/core/model";
import { useBotMap, useOutputs, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { CheckRow, Row, Section } from "../../../src/ui/forms";
import { daySeparator } from "../../../src/ui/format";
import { openFile, openLink, OutputPreview, outputAttachment, shareFile, shareLink } from "../../../src/ui/outputs";
import { Font, usePalette } from "../../../src/ui/theme";
import { canPreviewFiles } from "../../../modules/lorca-core";

export default function OutputScreen() {
  useLanguage();
  const { id, chat } = useLocalSearchParams<{ id: string; chat: string }>();
  const p = usePalette();
  const bots = useBotMap();
  const series = useOutputs(chat).find((each) => each.id === id);
  const [picked, setPicked] = useState<string>();
  const message = series?.versions.find((version) => version.id === picked) ?? series?.versions[0];
  const attachment = message ? outputAttachment(message) : undefined;
  const error = useStore((s) => (attachment ? s.fileErrors[attachment.id] : undefined));
  const fetched = useStore((s) => (attachment ? !!s.files[attachment.id] : false));
  if (!series || !message?.output) return null;

  const output = message.output;
  const link = documentUrl(output);
  const evidence = output.evidence;
  const published = `${bots.get(output.bot_id)?.name ?? t("A bot")} · ${daySeparator(new Date(message.created_at * 1000))}`;

  return (
    <>
      <Stack.Screen options={{ title: output.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        {attachment ? (
          <Section footer={published}>
            <OutputPreview key={attachment.id} attachment={attachment} />
          </Section>
        ) : link ? (
          <Section title={t("Link")} footer={published}>
            <Row title={new URL(link).host} subtitle={link} icon="link" chevron onPress={() => openLink(link)} />
          </Section>
        ) : null}

        {evidence && (
          <Section title={evidenceTitle(evidence)}>
            <Row title={t("Result")} accessory={<Text style={[styles.value, { color: evidence.status === "failed" ? p.red : p.secondaryLabel }]}>{evidenceStatus(evidence)}</Text>} />
            <View style={styles.summary}>
              <Text style={[styles.summaryText, { color: p.label }]} selectable>
                {evidence.summary}
              </Text>
            </View>
            {evidence.command ? (
              <View style={styles.command}>
                <Text style={[styles.value, { color: p.label }]}>{t("Command")}</Text>
                <Text style={[styles.mono, { color: p.secondaryLabel }]} selectable>
                  {evidence.command}
                </Text>
              </View>
            ) : null}
          </Section>
        )}

        {series.versions.length > 1 && (
          <Section title={t("Versions")}>
            {series.versions.map((version) => (
              <CheckRow
                key={version.id}
                title={t("Version {number}", { number: version.output!.version })}
                subtitle={[daySeparator(new Date(version.created_at * 1000)), version.output!.evidence ? evidenceStatus(version.output!.evidence) : null].filter(Boolean).join(" · ")}
                checked={version.id === message.id}
                onPress={() => setPicked(version.id === series.versions[0].id ? undefined : version.id)}
              />
            ))}
          </Section>
        )}

        <Section>
          {attachment && error ? (
            <Row title={t("Try Again")} icon="arrow.clockwise" onPress={() => engine.retryFile(attachment)} />
          ) : attachment && canPreviewFiles ? (
            <Row title={t("Open")} icon="eye" onPress={fetched ? () => void openFile(attachment) : undefined} />
          ) : link ? (
            <Row title={t("Open")} icon="safari" onPress={() => openLink(link)} />
          ) : null}
          {attachment ? (
            <Row title={Platform.OS === "ios" ? t("Share…") : t("Share")} icon="square.and.arrow.up" onPress={fetched ? () => void shareFile(attachment) : undefined} />
          ) : link ? (
            <Row title={Platform.OS === "ios" ? t("Share…") : t("Share")} icon="square.and.arrow.up" onPress={() => shareLink(link)} />
          ) : null}
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  value: { fontSize: Font.body },
  summary: { paddingHorizontal: 16, paddingVertical: 11 },
  summaryText: { fontSize: Font.body, lineHeight: 21 },
  command: { paddingHorizontal: 16, paddingVertical: 10, gap: 4 },
  mono: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 13 },
});
