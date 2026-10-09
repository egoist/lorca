// What a chat's bots published, after the macOS app's inspector: a row per output in its
// details, and an output's preview. Files come from the core, fetched from the relay when
// another Device published them; Open shows one in Quick Look on iOS, and Share hands it to the
// share sheet, which is where the phone saves a copy.

import { File } from "expo-file-system";
import { Image } from "expo-image";
import * as Sharing from "expo-sharing";
import * as WebBrowser from "expo-web-browser";
import { useEffect, useState } from "react";
import { Platform, ScrollView, Share, StyleSheet, Text, View } from "react-native";
import { canPreviewFiles, previewFile } from "../../modules/lorca-core";
import { engine } from "../core/engine";
import { evidenceStatus, fileSize, isImage, outputSymbol, type Attachment, type Message, type OutputSeries } from "../core/model";
import { useBotMap, useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { Row } from "./forms";
import { stamp } from "./format";
import { Pressable } from "./Pressable";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

/// A text file shows whole in the preview up to this size; a bigger one shows as a file.
const TEXT_PREVIEW_BYTES = 256 * 1024;

export function outputAttachment(message: Message): Attachment | undefined {
  return message.body.kind === "text" ? message.body.attachments?.[0] : undefined;
}

/// An output's row: what it is, its name, when it was published (and by whom, in a group), and
/// how its check went. It opens the output.
export function OutputRow({ series, isGroup, onPress }: { series: OutputSeries; isGroup: boolean; onPress: () => void }) {
  useLanguage();
  const p = usePalette();
  const bots = useBotMap();
  const latest = series.versions[0];
  const output = latest.output!;
  const when = stamp(new Date(latest.created_at * 1000));
  const bot = isGroup ? bots.get(output.bot_id)?.name : undefined;
  const evidence = output.evidence;
  return (
    <Row
      title={output.name}
      subtitle={bot ? `${bot} · ${when}` : when}
      icon={outputSymbol(output)}
      accessory={evidence ? <Text style={[styles.status, { color: evidence.status === "failed" ? p.red : p.secondaryLabel }]}>{evidenceStatus(evidence)}</Text> : undefined}
      chevron
      onPress={onPress}
    />
  );
}

/// The file in Quick Look on iOS; on Android, which has none, the share sheet.
export async function openFile(attachment: Attachment) {
  try {
    const path = await engine.namedFile(attachment);
    if (canPreviewFiles) await previewFile(path);
    else await Sharing.shareAsync(encodeURI(`file://${path}`), { mimeType: attachment.mime, dialogTitle: attachment.name });
  } catch (error) {
    fileFailed(error);
  }
}

/// The share sheet with the file under its own name: AirDrop, another app, or Save to Files.
export async function shareFile(attachment: Attachment) {
  try {
    const path = await engine.namedFile(attachment);
    await Sharing.shareAsync(encodeURI(`file://${path}`), { mimeType: attachment.mime, dialogTitle: attachment.name });
  } catch (error) {
    fileFailed(error);
  }
}

export function openLink(url: string) {
  void WebBrowser.openBrowserAsync(url);
}

export function shareLink(url: string) {
  void Share.share(Platform.OS === "ios" ? { url } : { message: url });
}

function fileFailed(error: unknown) {
  alert(t("Couldn't get this file"), error instanceof Error ? error.message : String(error));
}

/// The file as its sheet shows it: an image, the start of a text file, or what it is; while it is
/// on its way, or when it could not be fetched, it says so. A tap opens it on iOS.
export function OutputPreview({ attachment }: { attachment: Attachment }) {
  useLanguage();
  const p = usePalette();
  const uri = useStore((s) => s.files[attachment.id]);
  const error = useStore((s) => s.fileErrors[attachment.id]);
  useEffect(() => {
    if (!uri) void engine.fetchFile(attachment);
  }, [uri, attachment]);
  const text = useTextPreview(attachment, uri);

  let content: React.ReactNode;
  if (error) {
    content = (
      <View style={styles.placeholder}>
        <Text style={[styles.placeholderTitle, { color: p.secondaryLabel }]}>{t("This file couldn't be downloaded.")}</Text>
        <Text style={[styles.placeholderDetail, { color: p.tertiaryLabel }]}>{error.charAt(0).toUpperCase() + error.slice(1)}</Text>
      </View>
    );
  } else if (!uri) {
    content = (
      <View style={styles.placeholder}>
        <Text style={[styles.placeholderTitle, { color: p.secondaryLabel }]}>{t("Downloading…")}</Text>
      </View>
    );
  } else if (isImage(attachment)) {
    const ratio = attachment.width && attachment.height ? attachment.width / attachment.height : 4 / 3;
    content = <Image source={{ uri }} style={[styles.image, { aspectRatio: ratio }]} contentFit="contain" transition={150} />;
  } else if (text !== undefined) {
    content = (
      <ScrollView style={[styles.text, { backgroundColor: p.fill }]} nestedScrollEnabled>
        <Text style={[styles.mono, { color: p.label }]} selectable>
          {text}
        </Text>
      </ScrollView>
    );
  } else {
    content = (
      <View style={styles.placeholder}>
        <Symbol name="doc" size={40} color={p.secondaryLabel} />
        <Text style={[styles.fileName, { color: p.label }]} numberOfLines={2}>
          {attachment.name}
        </Text>
        <Text style={[styles.placeholderDetail, { color: p.secondaryLabel }]}>{fileSize(attachment.size)}</Text>
      </View>
    );
  }
  return (
    <Pressable onPress={uri && canPreviewFiles ? () => void openFile(attachment) : undefined} disabled={!uri || !canPreviewFiles} accessibilityLabel={attachment.name} accessibilityRole={uri && canPreviewFiles ? "button" : undefined}>
      {content}
    </Pressable>
  );
}

/// The words of a text file small enough to show whole.
function useTextPreview(attachment: Attachment, uri: string | undefined): string | undefined {
  const textLike = attachment.mime.startsWith("text/") || attachment.mime === "application/json" || attachment.mime === "application/xml";
  const [text, setText] = useState<{ uri: string; text: string }>();
  useEffect(() => {
    if (!uri || !textLike || attachment.size > TEXT_PREVIEW_BYTES) return;
    let live = true;
    new File(uri)
      .text()
      .then((words) => live && setText({ uri, text: words }))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [uri, textLike, attachment.size]);
  return text && text.uri === uri ? text.text : undefined;
}

const styles = StyleSheet.create({
  status: { fontSize: Font.body },
  placeholder: { height: 220, alignItems: "center", justifyContent: "center", gap: 6, paddingHorizontal: 24 },
  placeholderTitle: { fontSize: Font.body, textAlign: "center" },
  placeholderDetail: { fontSize: Font.small, textAlign: "center" },
  fileName: { fontSize: Font.body, fontWeight: "500", textAlign: "center", marginTop: 4 },
  image: { width: "100%", maxHeight: 300 },
  text: { height: 240 },
  mono: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12, lineHeight: 17, padding: 12 },
});
