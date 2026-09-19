// A bot's look, slid in from its avatar in Details inside the same sheet: a photo of the
// user's own, or one of the symbols on one of the accents. Every choice applies at once and reaches every paired Device
// through the roster; a photo travels as an encrypted `file` blob like an attachment.

import { ImageManipulator, SaveFormat } from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import { Stack, useLocalSearchParams } from "expo-router";
import { Alert, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { engine, type PickedFile } from "../../../src/core/engine";
import { useBotMap } from "../../../src/core/store";
import { t } from "../../../src/i18n";
import { AvatarDisc, useBotAvatarUri } from "../../../src/ui/Avatar";
import { Row, Section } from "../../../src/ui/forms";
import { BOT_SYMBOLS, Symbol } from "../../../src/ui/Symbol";
import { ACCENTS, accentColors, usePalette } from "../../../src/ui/theme";

export default function BotLookScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const p = usePalette();
  const bot = useBotMap().get(id);
  const uri = useBotAvatarUri(bot);

  if (!bot) return null;

  /// The system picker (instant, needs no permission), then a square center crop at
  /// `AVATAR_SIDE` px made here: the picker's own crop step is the legacy controller, which takes
  /// seconds to appear, and a full photo is far more than an avatar needs to sync.
  async function choosePhoto() {
    const result = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ["images"], quality: 1 });
    if (result.canceled) return;
    const asset = result.assets[0];
    try {
      const square = await squareAvatar(asset);
      await engine.setBotAvatar(bot!.id, square);
    } catch (error) {
      Alert.alert(t("Could not use that photo"), error instanceof Error ? error.message : String(error));
    }
  }

  function removePhoto() {
    void engine.setBotAvatar(bot!.id, null).catch((error) => Alert.alert(t("Could not remove the photo"), error instanceof Error ? error.message : String(error)));
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Look") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <View style={styles.hero}>
          <AvatarDisc symbol={bot.symbol_name} accent={bot.accent} uri={uri} size={96} />
        </View>
        <Section title={t("Photo")} footer={bot.avatar ? t("The photo shows in place of the symbol and color, on every paired Device.") : t("A photo shows in place of the symbol and color. It is shared encrypted, like an attachment.")}>
          <Row title={bot.avatar ? t("Change Photo") : t("Choose Photo")} icon="photo.on.rectangle" onPress={() => void choosePhoto()} />
          {bot.avatar ? <Row title={t("Remove Photo")} icon="xmark.circle.fill" destructive onPress={removePhoto} /> : null}
        </Section>
        <Section title={t("Symbol")}>
          <View style={styles.grid}>
            {BOT_SYMBOLS.map((s) => (
              <Pressable key={s} onPress={() => void engine.setBotLook(bot.id, { symbol_name: s })} style={[styles.symbolCell, { backgroundColor: s === bot.symbol_name ? accentColors(bot.accent, p.dark)[1] : p.fill }]} accessibilityLabel={s}>
                <Symbol name={s} size={20} color={s === bot.symbol_name ? "#FFFFFF" : p.label} />
              </Pressable>
            ))}
          </View>
        </Section>
        <Section title={t("Color")}>
          <View style={styles.grid}>
            {(Object.keys(ACCENTS) as (keyof typeof ACCENTS)[]).map((a) => (
              <Pressable key={a} onPress={() => void engine.setBotLook(bot.id, { accent: a })} style={styles.swatchCell} accessibilityLabel={a}>
                <View style={[styles.swatch, { backgroundColor: accentColors(a, p.dark)[1], borderColor: a === bot.accent ? p.label : "transparent" }]} />
              </Pressable>
            ))}
          </View>
        </Section>
      </ScrollView>
    </>
  );
}

/// Longest side of a stored profile image, the Mac app's figure too.
const AVATAR_SIDE = 512;

async function squareAvatar(asset: ImagePicker.ImagePickerAsset): Promise<PickedFile> {
  const side = Math.min(asset.width, asset.height);
  const context = ImageManipulator.manipulate(asset.uri);
  context.crop({ originX: Math.floor((asset.width - side) / 2), originY: Math.floor((asset.height - side) / 2), width: side, height: side });
  if (side > AVATAR_SIDE) context.resize({ width: AVATAR_SIDE, height: AVATAR_SIDE });
  const rendered = await context.renderAsync();
  const saved = await rendered.saveAsync({ format: SaveFormat.JPEG, compress: 0.85 });
  rendered.release();
  context.release();
  const base = (asset.fileName ?? "Photo").replace(/\.[^.]+$/, "");
  return { uri: saved.uri, name: `${base}.jpg`, mime: "image/jpeg", width: saved.width, height: saved.height };
}

const styles = StyleSheet.create({
  hero: { alignItems: "center", paddingTop: 16, paddingBottom: 8 },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 10, padding: 14 },
  symbolCell: { width: 40, height: 40, borderRadius: 10, alignItems: "center", justifyContent: "center" },
  swatchCell: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
  swatch: { width: 28, height: 28, borderRadius: 14, borderWidth: 2.5 },
});
