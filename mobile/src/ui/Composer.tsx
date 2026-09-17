// The message field, after Grok Bot's phone composer: a liquid-glass "+" button that drops down
// a native menu of attachment sources, and a glass pill that floats over the transcript, holding
// a multiline input that grows to five lines and, inside its right edge, a disc that is Dictate
// while the field is empty and Send once there is something to send. @-mention chips in a group. There is no Stop, as in Grok Bot: a turn runs
// to its end. Where liquid glass is not available (older iOS, Android) the pill is a plain
// filled field.

import { Button as MenuButton, Host, Image as MenuImage, Menu, type ButtonProps } from "@expo/ui/swift-ui";
import { background, frame, glassEffect } from "@expo/ui/swift-ui/modifiers";
import * as DocumentPicker from "expo-document-picker";
import { GlassView, isLiquidGlassAvailable } from "expo-glass-effect";
import * as Haptics from "expo-haptics";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { ExpoSpeechRecognitionModule, useSpeechRecognitionEvent } from "expo-speech-recognition";
import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Linking, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View, type ColorValue } from "react-native";
import type { PickedFile } from "../core/engine";
import { fileSize, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, type Bot } from "../core/model";
import { BotAvatar } from "./Avatar";
import { pickDictationLanguage, supportedLanguages, useDictationLanguage } from "./dictation";
import { joinDictation } from "./format";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

const MAX_LINES = 5;
const CHIP = 56;
const PLUS = 46;
const GLASS = isLiquidGlassAvailable();

/// A glass surface, or a filled one where glass is not available.
function Surface({ style, children, tint, edge }: { style: any; children: React.ReactNode; tint: ColorValue; edge: ColorValue }) {
  // A hairline edge keeps the pill visible over a plain background, as Grok Bot's is.
  const outline = { borderWidth: StyleSheet.hairlineWidth, borderColor: edge };
  if (GLASS) {
    // The glass is a layer under the content; the edge is drawn by the wrapping view.
    return (
      <View style={[style, outline]}>
        <GlassView glassEffectStyle="regular" isInteractive style={[StyleSheet.absoluteFill, { borderRadius: style.borderRadius }]} />
        {children}
      </View>
    );
  }
  return <View style={[style, outline, { backgroundColor: tint }]}>{children}</View>;
}

interface AttachSource {
  title: string;
  icon: NonNullable<ButtonProps["systemImage"]>;
  run: () => void;
}

/// The "+" disc as the label of a SwiftUI Menu, so a tap drops down a native menu of sources
/// right at the button instead of raising a sheet. The disc is glass where glass exists and a
/// filled circle elsewhere, matching Surface.
function AttachMenu({ sources, tint, label, edge }: { sources: AttachSource[]; tint: ColorValue; label: ColorValue; edge: ColorValue }) {
  const disc = GLASS
    ? glassEffect({ glass: { variant: "regular", interactive: true }, shape: "circle" })
    : background(String(tint));
  return (
    <View style={[styles.plus, { borderWidth: StyleSheet.hairlineWidth, borderColor: edge }]}>
      <Host style={StyleSheet.absoluteFill} testID="attach">
        <Menu
          label={<MenuImage systemName="plus" size={22} color={label} modifiers={[frame({ width: PLUS, height: PLUS }), disc]} />}
          modifiers={[frame({ width: PLUS, height: PLUS })]}
        >
          {sources.map((source) => (
            <MenuButton key={source.title} systemImage={source.icon} label={source.title} onPress={source.run} />
          ))}
        </Menu>
      </Host>
    </View>
  );
}

export function Composer({
  members,
  isGroup,
  placeholder,
  onSend,
}: {
  members: Bot[];
  isGroup: boolean;
  placeholder: string;
  onSend: (text: string, attachments: PickedFile[]) => void;
}) {
  const p = usePalette();
  const [text, setText] = useState("");
  const [height, setHeight] = useState(0);
  const [attachments, setAttachments] = useState<PickedFile[]>([]);
  const [listening, setListening] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [levels, setLevels] = useState<number[]>([0, 0, 0, 0, 0]);
  /// The transcript accumulates while the pill shows; it lands in the field when the user
  /// stops or sends, the way Grok Bot commits a recording.
  const transcript = useRef("");
  const pendingSend = useRef(false);
  const { language } = useDictationLanguage();
  const canSend = text.trim().length > 0 || attachments.length > 0;
  const lineHeight = Font.body * 1.3;

  const mention = useMemo(() => {
    if (!isGroup) return null;
    const match = /(?:^|\s)@(\w*)$/.exec(text);
    if (!match) return null;
    const prefix = match[1].toLowerCase();
    const matches = members.filter((b) => b.name.toLowerCase().startsWith(prefix));
    return matches.length ? { prefix: match[1], matches } : null;
  }, [text, isGroup, members]);

  function insertMention(bot: Bot) {
    setText((t) => t.replace(/@(\w*)$/, `@${bot.name} `));
  }

  function send() {
    if (listening) {
      // The recording ends, its words land in the field, and the message goes.
      pendingSend.current = true;
      ExpoSpeechRecognitionModule.stop();
      return;
    }
    if (!canSend) return;
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onSend(text, attachments);
    setText("");
    setAttachments([]);
    setHeight(0);
  }

  // MARK: - Attachments

  function addFiles(files: PickedFile[]) {
    const problems: string[] = [];
    setAttachments((current) => {
      const next = [...current];
      for (const file of files) {
        if (next.length >= MAX_ATTACHMENTS) {
          problems.push(`At most ${MAX_ATTACHMENTS} files per message.`);
          break;
        }
        if (file.size !== undefined && file.size > MAX_ATTACHMENT_BYTES) {
          problems.push(`${file.name} is larger than ${MAX_ATTACHMENT_BYTES / 1024 / 1024} MB.`);
          continue;
        }
        next.push(file);
      }
      return next;
    });
    if (problems.length) Alert.alert("Some files were not attached", problems.join("\n"));
  }

  async function pickPhotos() {
    const result = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ["images"],
      allowsMultipleSelection: true,
      selectionLimit: Math.max(1, MAX_ATTACHMENTS - attachments.length),
      quality: 0.85,
    });
    if (result.canceled) return;
    addFiles(result.assets.map(imageAsset));
  }

  async function takePhoto() {
    const permission = await ImagePicker.requestCameraPermissionsAsync();
    if (!permission.granted) {
      Alert.alert("Camera access is off", "Allow the camera for Tinybot in Settings to take a photo.", [
        { text: "Settings", onPress: () => void Linking.openSettings() },
        { text: "OK", style: "cancel" },
      ]);
      return;
    }
    const result = await ImagePicker.launchCameraAsync({ mediaTypes: ["images"], quality: 0.85 });
    if (result.canceled) return;
    addFiles(result.assets.map(imageAsset));
  }

  async function pickFiles() {
    const result = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true });
    if (result.canceled) return;
    addFiles(
      result.assets.map((asset) => ({
        uri: asset.uri,
        name: asset.name,
        mime: asset.mimeType ?? "application/octet-stream",
        size: asset.size ?? undefined,
      })),
    );
  }

  const sources: AttachSource[] = [
    { title: "Photo Library", icon: "photo.on.rectangle", run: () => void pickPhotos() },
    { title: "Take Photo", icon: "camera", run: () => void takePhoto() },
    { title: "Choose File", icon: "folder", run: () => void pickFiles() },
  ];

  // Android has no native pull-down menu here; a dialog lists the same sources.
  function attachDialog() {
    Alert.alert("Attach", undefined, [...sources.map((a) => ({ text: a.title, onPress: a.run })), { text: "Cancel", style: "cancel" as const }]);
  }

  // MARK: - Dictation

  useEffect(() => {
    void supportedLanguages();
  }, []);

  useEffect(() => {
    if (!listening) return;
    const started = Date.now();
    setElapsed(0);
    const timer = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 500);
    return () => clearInterval(timer);
  }, [listening]);

  useSpeechRecognitionEvent("result", (event) => {
    transcript.current = event.results[0]?.transcript ?? "";
  });
  useSpeechRecognitionEvent("volumechange", (event) => {
    // -2…10 from the recognizer; anything under 0 is silence.
    const level = Math.min(1, Math.max(0, event.value / 8));
    setLevels((current) => [...current.slice(1), level]);
  });
  useSpeechRecognitionEvent("end", () => finishDictation());
  useSpeechRecognitionEvent("error", (event) => {
    const problem = event.error === "aborted" || event.error === "no-speech" ? null : event.message;
    finishDictation(problem);
  });

  function finishDictation(problem: string | null = null) {
    setListening(false);
    setLevels([0, 0, 0, 0, 0]);
    const words = transcript.current.trim();
    transcript.current = "";
    const next = words ? joinDictation(text, words) : text;
    if (words) setText(next);
    if (problem) {
      pendingSend.current = false;
      Alert.alert("Dictation stopped", problem);
      return;
    }
    if (pendingSend.current) {
      pendingSend.current = false;
      if (next.trim() || attachments.length) {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onSend(next, attachments);
        setText("");
        setAttachments([]);
        setHeight(0);
      }
    }
  }

  async function dictate() {
    if (listening) {
      ExpoSpeechRecognitionModule.stop();
      return;
    }
    const permission = await ExpoSpeechRecognitionModule.requestPermissionsAsync();
    if (!permission.granted) {
      Alert.alert("Dictation needs the microphone", "Allow the microphone and speech recognition for Tinybot in Settings.", [
        { text: "Settings", onPress: () => void Linking.openSettings() },
        { text: "OK", style: "cancel" },
      ]);
      return;
    }
    transcript.current = "";
    pendingSend.current = false;
    setListening(true);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    ExpoSpeechRecognitionModule.start({
      lang: language,
      interimResults: true,
      continuous: true,
      addsPunctuation: true,
      volumeChangeEventOptions: { enabled: true, intervalMillis: 100 },
    });
  }

  const primary = listening || canSend ? "send" : "dictate";
  const edge = p.dark ? "rgba(255,255,255,0.14)" : "rgba(0,0,0,0.1)";

  return (
    <View style={styles.wrap} pointerEvents="box-none">
      {mention && (
        <ScrollView horizontal keyboardShouldPersistTaps="always" showsHorizontalScrollIndicator={false} contentContainerStyle={styles.chips}>
          {mention.matches.map((bot) => (
            <Pressable key={bot.id} onPress={() => insertMention(bot)} style={({ pressed }) => [styles.chip, { backgroundColor: pressed ? p.secondaryFill : p.cell, opacity: pressed ? 0.8 : 1 }]}>
              <BotAvatar bot={bot} size={20} />
              <Text style={[styles.chipText, { color: p.label }]}>{bot.name}</Text>
            </Pressable>
          ))}
        </ScrollView>
      )}
      {attachments.length > 0 && (
        <ScrollView horizontal keyboardShouldPersistTaps="always" showsHorizontalScrollIndicator={false} contentContainerStyle={styles.files}>
          {attachments.map((file, index) => (
            <View key={`${file.uri}-${index}`} style={styles.fileChip}>
              {file.mime.startsWith("image/") ? (
                <Image source={{ uri: file.uri }} style={[styles.thumb, { backgroundColor: p.fill }]} contentFit="cover" />
              ) : (
                <View style={[styles.fileCard, { backgroundColor: p.cell }]}>
                  <Symbol name="doc.fill" size={18} color={p.secondaryLabel} />
                  <View style={{ flex: 1 }}>
                    <Text style={[styles.fileName, { color: p.label }]} numberOfLines={1}>
                      {file.name}
                    </Text>
                    {file.size !== undefined && <Text style={[styles.fileSize, { color: p.secondaryLabel }]}>{fileSize(file.size)}</Text>}
                  </View>
                </View>
              )}
              <Pressable
                onPress={() => setAttachments((current) => current.filter((_, i) => i !== index))}
                hitSlop={8}
                style={styles.remove}
                accessibilityLabel={`Remove ${file.name}`}
              >
                <Symbol name="xmark" size={9} color="#FFFFFF" weight="bold" />
              </Pressable>
            </View>
          ))}
        </ScrollView>
      )}
      <View style={styles.bar}>
        {Platform.OS === "ios" ? (
          <AttachMenu sources={sources} tint={p.cell} label={p.label} edge={edge} />
        ) : (
          <Surface style={styles.plus} tint={p.cell} edge={edge}>
            <Pressable onPress={attachDialog} style={({ pressed }) => [styles.plusPress, { opacity: pressed ? 0.6 : 1 }]} accessibilityLabel="Attach">
              <Symbol name="plus" size={22} color={p.label} weight="medium" />
            </Pressable>
          </Surface>
        )}
        <Surface style={styles.field} tint={p.cell} edge={edge}>
          {listening ? (
            <Pressable
              onPress={() => ExpoSpeechRecognitionModule.stop()}
              style={({ pressed }) => [styles.pill, { backgroundColor: p.fill, opacity: pressed ? 0.7 : 1 }]}
              accessibilityRole="button"
              accessibilityLabel={`Stop recording, ${elapsed} seconds`}
            >
              <View style={styles.stop}>
                <Symbol name="stop.fill" size={11} color={p.label} weight="bold" />
              </View>
              <Text style={[styles.elapsed, { color: p.label }]}>{`${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`}</Text>
              <View style={styles.bars}>
                {levels.map((level, index) => (
                  <View key={index} style={[styles.levelBar, { backgroundColor: p.label, height: 3 + level * 11 }]} />
                ))}
              </View>
            </Pressable>
          ) : (
            <TextInput
              value={text}
              onChangeText={setText}
              placeholder={placeholder}
              placeholderTextColor={p.tertiaryLabel}
              multiline
              keyboardAppearance={p.dark ? "dark" : "light"}
              onContentSizeChange={(e) => setHeight(e.nativeEvent.contentSize.height)}
              style={[styles.input, { color: p.label, lineHeight, height: Math.min(MAX_LINES, Math.max(1, Math.round(height / lineHeight))) * lineHeight }]}
              accessibilityLabel="Message"
            />
          )}
          <Pressable
            onPress={primary === "send" ? send : () => void dictate()}
            onLongPress={primary === "dictate" ? () => void pickDictationLanguage() : undefined}
            style={({ pressed }) => [styles.disc, { backgroundColor: primary === "send" ? p.tint : p.fill, opacity: pressed ? 0.7 : 1 }]}
            accessibilityLabel={primary === "send" ? "Send" : "Dictate"}
            accessibilityHint={primary === "dictate" ? "Long press to choose the language" : undefined}
          >
            <Symbol name={primary === "send" ? "arrow.up" : "mic.fill"} size={16} color={primary === "send" ? "#FFFFFF" : p.label} weight="bold" />
          </Pressable>
        </Surface>
      </View>
    </View>
  );
}

function imageAsset(asset: ImagePicker.ImagePickerAsset): PickedFile {
  const name = asset.fileName ?? `Photo ${new Date().toISOString().slice(0, 19).replace("T", " ").replace(/:/g, ".")}.jpg`;
  return {
    uri: asset.uri,
    name,
    mime: asset.mimeType ?? (name.toLowerCase().endsWith(".png") ? "image/png" : "image/jpeg"),
    size: asset.fileSize ?? undefined,
    width: asset.width,
    height: asset.height,
  };
}

function deviceLanguage(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().locale || "en-US";
  } catch {
    return "en-US";
  }
}

const styles = StyleSheet.create({
  wrap: { paddingBottom: 8 },
  bar: { flexDirection: "row", alignItems: "flex-end", paddingHorizontal: 12, gap: 10 },
  plus: { width: PLUS, height: PLUS, borderRadius: PLUS / 2, overflow: "hidden" },
  plusPress: { flex: 1, alignItems: "center", justifyContent: "center" },
  field: { flex: 1, flexDirection: "row", alignItems: "flex-end", borderRadius: 23, height: undefined, paddingLeft: 18, paddingRight: 6, paddingVertical: 6, overflow: "hidden" },
  // One line of text makes the pill exactly as tall as the + disc: 6 + 22 + 6 inside 6 + 6 of padding.
  input: { flex: 1, fontSize: Font.body, paddingTop: 0, paddingBottom: 0, margin: 0, marginVertical: 6 },
  // At the right of the field, beside Send, where Grok Bot puts it.
  pill: { flexDirection: "row", alignItems: "center", gap: 6, height: 28, borderRadius: 14, paddingLeft: 4, paddingRight: 10, marginLeft: "auto", marginRight: 6, marginBottom: 3 },
  stop: { width: 22, height: 22, alignItems: "center", justifyContent: "center" },
  elapsed: { fontSize: 14, fontVariant: ["tabular-nums"] },
  bars: { flexDirection: "row", alignItems: "center", gap: 2.5, height: 14 },
  levelBar: { width: 3, borderRadius: 1.5, opacity: 0.85 },
  disc: { width: 34, height: 34, borderRadius: 17, alignItems: "center", justifyContent: "center" },
  chips: { paddingHorizontal: 12, paddingBottom: 8, gap: 8 },
  chip: { flexDirection: "row", alignItems: "center", gap: 6, paddingLeft: 4, paddingRight: 10, paddingVertical: 4, borderRadius: 14 },
  chipText: { fontSize: 14, fontWeight: "500" },
  files: { paddingHorizontal: 12, paddingTop: 6, paddingBottom: 10, gap: 10 },
  fileChip: { height: CHIP },
  thumb: { width: CHIP, height: CHIP, borderRadius: 10 },
  fileCard: { height: CHIP, width: 170, borderRadius: 10, flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 10 },
  fileName: { fontSize: 13, fontWeight: "500" },
  fileSize: { fontSize: Font.caption },
  remove: { position: "absolute", top: -6, right: -6, width: 18, height: 18, borderRadius: 9, backgroundColor: "rgba(0,0,0,0.7)", alignItems: "center", justifyContent: "center" },
});
