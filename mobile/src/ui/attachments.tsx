// Attachments in a bubble: image thumbnails sized from the width and height the message
// carries, and a card for any other file, which opens it. The bytes come from the phone's own
// store, or from the relay the first time a message from another Device shows them; a fetch that
// failed says so and is tried again on a tap.

import { Image } from "expo-image";
import { useRouter } from "expo-router";
import { useEffect } from "react";
import { StyleSheet, Text, View, type ColorValue } from "react-native";
import { Pressable } from "./Pressable";
import { engine } from "../core/engine";
import { fileSize, isImage, type Attachment } from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";
import { openFile } from "./outputs";

export const IMAGE_MAX = 220;
const IMAGE_MIN = 72;
const FILE_WIDTH = 220;

/// The file URI for an attachment, once this phone has the bytes.
export function useAttachmentUri(attachment: Attachment): string | undefined {
  const uri = useStore((s) => s.files[attachment.id]);
  useEffect(() => {
    if (!uri) void engine.fetchFile(attachment);
  }, [uri, attachment]);
  return uri;
}

/// Thumbnail box for an image, from its pixel size; a 4:3 box when the size is unknown.
export function imageBox(attachment: Attachment, maxWidth: number): { width: number; height: number } {
  const w = Math.max(attachment.width ?? 4, 1);
  const h = Math.max(attachment.height ?? 3, 1);
  const scale = Math.min(IMAGE_MAX / w, IMAGE_MAX / h, maxWidth / w, 1);
  return { width: Math.max(IMAGE_MIN, Math.ceil(w * scale)), height: Math.max(IMAGE_MIN, Math.ceil(h * scale)) };
}

export function AttachmentBlock({ attachments, onUserBubble, maxWidth }: { attachments: Attachment[]; onUserBubble: boolean; maxWidth: number }) {
  return (
    <View style={styles.block}>
      {attachments.map((attachment) => (
        <AttachmentTile key={attachment.id} attachment={attachment} onUserBubble={onUserBubble} maxWidth={maxWidth} />
      ))}
    </View>
  );
}

function AttachmentTile({ attachment, onUserBubble, maxWidth }: { attachment: Attachment; onUserBubble: boolean; maxWidth: number }) {
  useLanguage();
  const p = usePalette();
  const router = useRouter();
  const uri = useAttachmentUri(attachment);
  const error = useStore((s) => s.fileErrors[attachment.id]);
  const quiet: ColorValue = onUserBubble ? "rgba(255,255,255,0.18)" : p.fill;
  const foreground: ColorValue = onUserBubble ? p.userBubbleText : p.botBubbleText;

  if (isImage(attachment) && !error) {
    const box = imageBox(attachment, maxWidth);
    return (
      <Pressable
        onPress={() => uri && router.push({ pathname: "/attachment/[id]", params: { id: attachment.id, name: attachment.name } })}
        accessibilityLabel={attachment.name}
        accessibilityRole="imagebutton"
      >
        <Image source={uri ? { uri } : undefined} style={[box, styles.image, { backgroundColor: quiet }]} contentFit="cover" transition={150} />
      </Pressable>
    );
  }
  // A file's card opens it, as its sheet's Open does; one whose bytes could not be fetched says so
  // and fetches them again on a tap.
  const detail = error ? t("Couldn't download · Retry") : uri ? fileSize(attachment.size) : `${fileSize(attachment.size)} · ${t("fetching…")}`;
  return (
    <Pressable
      onPress={error ? () => engine.retryFile(attachment) : uri ? () => void openFile(attachment) : undefined}
      disabled={!error && !uri}
      style={[styles.card, { backgroundColor: quiet, width: Math.min(FILE_WIDTH, maxWidth) }, isImage(attachment) && imageBox(attachment, maxWidth)]}
      accessibilityLabel={`${attachment.name}, ${detail}`}
      accessibilityRole="button"
    >
      <Symbol name={isImage(attachment) ? "photo" : "doc.fill"} size={22} color={foreground} />
      <View style={{ flex: 1 }}>
        <Text style={[styles.name, { color: foreground }]} numberOfLines={1}>
          {attachment.name}
        </Text>
        <Text style={[styles.size, { color: foreground, opacity: 0.7 }]} numberOfLines={1}>
          {detail}
        </Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  block: { flexDirection: "row", flexWrap: "wrap", gap: 6, marginBottom: 4 },
  image: { borderRadius: 12 },
  card: { flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 10, paddingVertical: 8, borderRadius: 12 },
  name: { fontSize: Font.body - 1, fontWeight: "500" },
  size: { fontSize: Font.caption },
});
