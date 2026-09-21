// The message field, laid out like the desktop composer: one glass pill floating over the
// transcript. Inside its left edge a "+" disc drops down a native menu of attachment sources;
// inside its right edge a disc is Dictate while the field is empty and Send once there is
// something to send; between them a multiline input grows to five lines. Focus, text, or
// attached files expand the pill: the text takes its own row above the discs, and chips wrap
// above the text. @-mention chips in a group. There is no hard Stop on the phone; sending a
// message while a turn runs steers it. Where liquid glass is not available (older iOS, Android)
// the pill is a plain filled field.

import { Button as MenuButton, Host, Image as MenuImage, Menu, type ButtonProps } from "@expo/ui/swift-ui";
import { background, frame, shapes } from "@expo/ui/swift-ui/modifiers";
import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import * as DocumentPicker from "expo-document-picker";
import { GlassView, isLiquidGlassAvailable } from "expo-glass-effect";
import * as Haptics from "expo-haptics";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { ExpoSpeechRecognitionModule, useSpeechRecognitionEvent } from "expo-speech-recognition";
import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Linking, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View, type ColorValue, type ImageSourcePropType, type StyleProp, type ViewStyle } from "react-native";
import type { PickedFile } from "../core/engine";
import { fileSize, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, type Bot } from "../core/model";
import { BotAvatar } from "./Avatar";
import { automaticLanguage, languageName, pickDictationLanguage, setDictationLanguage, useDictationLanguage, useSupportedLanguages } from "./dictation";
import { t } from "../i18n";
import { joinDictation } from "./format";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";
import { AndroidIcons } from "./navigation";

const MAX_LINES = 5;
const CHIP = 56;
const FILE_CHIP = 176;
const DISC = 34;
const GLASS = isLiquidGlassAvailable();

/// A glass surface, or a filled one where glass is not available.
export function Surface({ style, children, tint, edge, onPress }: { style: StyleProp<ViewStyle>; children: React.ReactNode; tint: ColorValue; edge: ColorValue; onPress?: () => void }) {
  // A hairline edge keeps the pill visible over a plain background, as Grok Bot's is.
  const outline = { borderWidth: StyleSheet.hairlineWidth, borderColor: edge };
  if (GLASS) {
    // The glass is a layer under the content; the edge is drawn by the wrapping view.
    return (
      <Pressable onPress={onPress} disabled={!onPress} accessible={false} style={[style, outline]}>
        <GlassView glassEffectStyle="regular" isInteractive style={[StyleSheet.absoluteFill, { borderRadius: StyleSheet.flatten(style)?.borderRadius }]} />
        {children}
      </Pressable>
    );
  }
  return (
    <Pressable onPress={onPress} disabled={!onPress} accessible={false} style={[style, outline, { backgroundColor: tint }]}>
      {children}
    </Pressable>
  );
}

interface AttachSource {
  title: string;
  icon: NonNullable<ButtonProps["systemImage"]>;
  androidIcon: ImageSourcePropType;
  run: () => void;
}

/// The "+" disc as the label of a SwiftUI Menu, so a tap drops down a native menu of sources
/// right at the button instead of raising a sheet. The disc is filled like the Dictate disc
/// beside it; the pill under both supplies the glass. The fill is part of the SwiftUI label,
/// not the wrapping view: the menu morphs out of its label and hides it while open, so a
/// label that is only the glyph would leave an empty disc behind.
function AttachMenu({ sources, tint, label }: { sources: AttachSource[]; tint: ColorValue; label: ColorValue }) {
  return (
    <View style={styles.disc}>
      {/* The hosted view would otherwise avoid the keyboard by itself: SwiftUI treats the keys as
          a safe-area inset and pushes the disc up out of its frame while the sticky composer
          already rides above them. */}
      <Host style={StyleSheet.absoluteFill} ignoreSafeArea="keyboard" testID="attach">
        <Menu
          label={
            <MenuImage
              systemName="plus"
              size={18}
              color={label}
              modifiers={[frame({ width: DISC, height: DISC }), background(tint, shapes.circle())]}
            />
          }
          modifiers={[frame({ width: DISC, height: DISC })]}
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
  const [attachments, setAttachments] = useState<PickedFile[]>([]);
  const [focused, setFocused] = useState(false);
  const [listening, setListening] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [levels, setLevels] = useState<number[]>([0, 0, 0, 0, 0]);
  /// The transcript accumulates while the pill shows; it lands in the field when the user
  /// stops or sends, the way Grok Bot commits a recording.
  const transcript = useRef("");
  const pendingSend = useRef(false);
  const inputRef = useRef<TextInput>(null);
  const dictationMenuRef = useRef<MenuComponentRef>(null);
  const { language, setting: dictationSetting } = useDictationLanguage();
  const dictationLanguages = useSupportedLanguages();
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
    setText((current) => current.replace(/@(\w*)$/, `@${bot.name} `));
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
  }

  // MARK: - Attachments

  function addFiles(files: PickedFile[]) {
    const problems: string[] = [];
    setAttachments((current) => {
      const next = [...current];
      for (const file of files) {
        if (next.length >= MAX_ATTACHMENTS) {
          problems.push(t("At most {count} files per message.", { count: MAX_ATTACHMENTS }));
          break;
        }
        if (file.size !== undefined && file.size > MAX_ATTACHMENT_BYTES) {
          problems.push(t("{name} is larger than {size} MB.", { name: file.name, size: MAX_ATTACHMENT_BYTES / 1024 / 1024 }));
          continue;
        }
        next.push(file);
      }
      return next;
    });
    if (problems.length) Alert.alert(t("Some files were not attached"), problems.join("\n"));
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
      Alert.alert(t("Camera access is off"), t("Allow the camera for Lorca in Settings to take a photo."), [
        { text: t("Settings"), onPress: () => void Linking.openSettings() },
        { text: t("OK"), style: "cancel" },
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
    { title: t("Photo Library"), icon: "photo.on.rectangle", androidIcon: AndroidIcons.photos, run: () => void pickPhotos() },
    { title: t("Take Photo"), icon: "camera", androidIcon: AndroidIcons.camera, run: () => void takePhoto() },
    { title: t("Choose File"), icon: "folder", androidIcon: AndroidIcons.folder, run: () => void pickFiles() },
  ];

  // MARK: - Dictation

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
      Alert.alert(t("Dictation stopped"), problem);
      return;
    }
    if (pendingSend.current) {
      pendingSend.current = false;
      if (next.trim() || attachments.length) {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onSend(next, attachments);
        setText("");
        setAttachments([]);
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
      Alert.alert(t("Dictation needs the microphone"), t("Allow the microphone and speech recognition for Lorca in Settings."), [
        { text: t("Settings"), onPress: () => void Linking.openSettings() },
        { text: t("OK"), style: "cancel" },
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
  const expanded = focused || text.length > 0 || attachments.length > 0;

  // The pieces below carry keys and always share one parent view, so switching between the
  // compact row and the expanded stack reorders them instead of remounting them: a remounted
  // input drops focus and takes the keyboard down with it.
  const plus =
    Platform.OS === "ios" ? (
      <AttachMenu key="plus" sources={sources} tint={p.fill} label={p.label} />
    ) : (
      <MenuView
        key="plus"
        actions={sources.map((source, index) => ({ id: String(index), title: source.title, image: source.androidIcon }))}
        onPressAction={({ nativeEvent }) => sources[Number(nativeEvent.event)]?.run()}
        style={styles.androidDiscHost}
      >
        <View style={[styles.disc, styles.androidDisc, { backgroundColor: p.fill }]} accessible accessibilityRole="button" accessibilityLabel={t("Attach")}>
          <Symbol name="plus" size={18} color={p.label} weight="medium" />
        </View>
      </MenuView>
    );

  const recording = (
    <Pressable
      key="recording"
      onPress={() => ExpoSpeechRecognitionModule.stop()}
      style={({ pressed }) => [styles.pill, { backgroundColor: p.fill, opacity: pressed ? 0.7 : 1 }]}
      accessibilityRole="button"
      accessibilityLabel={t("Stop recording, {seconds} seconds", { seconds: elapsed })}
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
  );

  // The field sizes itself to its text (Fabric measures a multiline input with no fixed height)
  // up to five lines, past which the text view scrolls. A content-size event is no use here:
  // Fabric emits it only when the layout changes, so a fixed height never learns of a wrapped
  // line. The measurement leaves out a trailing empty line, so the text's own line count is a
  // floor: Return at the end of the text opens a new line at once.
  const breaks = text.split("\n").length;
  const minHeight = Math.min(MAX_LINES, Math.max(1, breaks)) * lineHeight;
  const maxHeight = MAX_LINES * lineHeight;
  const input = (
    <TextInput
      key="input"
      ref={inputRef}
      value={text}
      onChangeText={setText}
      placeholder={placeholder}
      placeholderTextColor={p.tertiaryLabel}
      multiline
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      keyboardAppearance={p.dark ? "dark" : "light"}
      style={[styles.input, expanded && styles.inputExpanded, { color: p.label, lineHeight, minHeight, maxHeight }]}
      accessibilityLabel={t("Message")}
    />
  );

  const primaryButton = (
    <Pressable
      key="primary"
      onPress={primary === "send" ? send : () => void dictate()}
      onLongPress={
        primary !== "dictate"
          ? undefined
          : Platform.OS === "ios"
            ? () => void pickDictationLanguage()
            : () => dictationMenuRef.current?.show()
      }
      style={({ pressed }) => [styles.disc, Platform.OS === "android" && styles.androidDisc, { backgroundColor: primary === "send" ? p.tint : p.fill, opacity: pressed ? 0.7 : 1 }]}
      accessibilityLabel={primary === "send" ? t("Send") : t("Dictate")}
      accessibilityHint={primary === "dictate" ? t("Long press to choose the language") : undefined}
    >
      <Symbol name={primary === "send" ? "arrow.up" : "mic.fill"} size={16} color={primary === "send" ? p.userBubbleText : p.label} weight="bold" />
    </Pressable>
  );
  const dictationActions: MenuAction[] = [
    {
      id: "automatic",
      title: t("Automatic ({language})", { language: languageName(automaticLanguage(dictationLanguages)) }),
      state: dictationSetting ? "off" : "on",
    },
    ...dictationLanguages.map((tag) => ({ id: tag, title: languageName(tag), state: dictationSetting === tag ? ("on" as const) : ("off" as const) })),
  ];
  const primaryDisc =
    Platform.OS === "android" && primary === "dictate" ? (
      <MenuView
        key="primary"
        ref={dictationMenuRef}
        actions={dictationActions}
        shouldOpenOnLongPress
        onPressAction={({ nativeEvent }) => setDictationLanguage(nativeEvent.event === "automatic" ? undefined : nativeEvent.event)}
        style={styles.androidDiscHost}
      >
        {primaryButton}
      </MenuView>
    ) : (
      primaryButton
    );

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
      {/* Focused, holding text, or carrying attachments, the pill expands: chips wrap above the
          text and the discs drop to a row underneath, as on the desktop. Empty and idle, the
          discs sit beside the placeholder. The input is one line tall inside a taller pill, so
          a tap anywhere on the pill outside the discs focuses it. */}
      <Surface style={[styles.field, expanded ? styles.fieldExpanded : styles.fieldCompact]} tint={p.cell} edge={edge} onPress={() => inputRef.current?.focus()}>
        {attachments.length > 0 && (
          <View style={styles.files}>
            {attachments.map((file, index) => (
              <View key={`${file.uri}-${index}`} style={styles.fileChip}>
                {file.mime.startsWith("image/") ? (
                  <Image source={{ uri: file.uri }} style={[styles.thumb, { backgroundColor: p.fill }]} contentFit="cover" />
                ) : (
                  <View style={[styles.fileCard, { backgroundColor: p.fill }]}>
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
                  accessibilityLabel={t("Remove {name}", { name: file.name })}
                >
                  <Symbol name="xmark" size={9} color="#FFFFFF" weight="bold" />
                </Pressable>
              </View>
            ))}
          </View>
        )}
        <View style={expanded ? styles.stack : styles.row}>
          {expanded
            ? [
                !listening && input,
                <View key="controls" style={styles.controls}>
                  {plus}
                  {listening && recording}
                  {primaryDisc}
                </View>,
              ]
            : [plus, listening ? recording : input, primaryDisc]}
        </View>
      </Surface>
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
  field: { marginHorizontal: 12, borderRadius: 23, overflow: "hidden" },
  // One line of text makes the pill exactly as tall as a disc plus its padding: 6 + 34 + 6.
  fieldCompact: { paddingHorizontal: 6, paddingVertical: 6 },
  fieldExpanded: { paddingHorizontal: 6, paddingTop: 0, paddingBottom: 6 },
  row: { flexDirection: "row", alignItems: "flex-end", gap: 6 },
  stack: { flexDirection: "column", alignItems: "stretch" },
  // Chips wrap; the padding leaves room for the remove button hanging off a chip's corner.
  files: { flexDirection: "row", flexWrap: "wrap", gap: 12, paddingTop: 12, paddingHorizontal: 6, paddingBottom: 2 },
  controls: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 6, paddingTop: 4 },
  input: { flex: 1, fontSize: Font.body, paddingTop: 0, paddingBottom: 0, margin: 0, marginVertical: 6, marginHorizontal: 6 },
  // In the stack the input is a row of its own: no growing into the column, wider margins.
  inputExpanded: { flex: 0, marginHorizontal: 12, marginTop: 10, marginBottom: 6 },
  // At the right of the field, beside Send, where Grok Bot puts it.
  pill: { flexDirection: "row", alignItems: "center", gap: 6, height: 28, borderRadius: 14, paddingLeft: 4, paddingRight: 10, marginLeft: "auto", marginBottom: 3 },
  stop: { width: 22, height: 22, alignItems: "center", justifyContent: "center" },
  elapsed: { fontSize: 14, fontVariant: ["tabular-nums"] },
  bars: { flexDirection: "row", alignItems: "center", gap: 2.5, height: 14 },
  levelBar: { width: 3, borderRadius: 1.5, opacity: 0.85 },
  disc: { width: DISC, height: DISC, borderRadius: DISC / 2, alignItems: "center", justifyContent: "center", overflow: "hidden" },
  androidDisc: { width: DISC - 1, height: DISC - 1, borderRadius: (DISC - 1) / 2 },
  androidDiscHost: { width: DISC + 1, height: DISC + 1, alignItems: "center", justifyContent: "center" },
  chips: { paddingHorizontal: 12, paddingBottom: 8, gap: 8 },
  chip: { flexDirection: "row", alignItems: "center", gap: 6, paddingLeft: 4, paddingRight: 10, paddingVertical: 4, borderRadius: 14 },
  chipText: { fontSize: 14, fontWeight: "500" },
  fileChip: { height: CHIP, maxWidth: "100%" },
  thumb: { width: CHIP, height: CHIP, borderRadius: 10 },
  fileCard: { height: CHIP, width: FILE_CHIP, maxWidth: "100%", borderRadius: 10, flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 10 },
  fileName: { fontSize: 13, fontWeight: "500" },
  fileSize: { fontSize: Font.caption },
  remove: { position: "absolute", top: -6, right: -6, width: 18, height: 18, borderRadius: 9, backgroundColor: "rgba(0,0,0,0.7)", alignItems: "center", justifyContent: "center" },
});
