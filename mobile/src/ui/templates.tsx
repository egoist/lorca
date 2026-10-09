// The pieces the template screens share, after the macOS app's TemplateItemViews: a row per
// piece of the bot, as the template holds it, with what to look at before sharing it, and the
// contents of a template read-only.

import { Checkbox, Host as ComposeHost } from "@expo/ui/jetpack-compose";
import { Platform, PlatformColor, StyleSheet, Text, View } from "react-native";
import { flagWords, memoryLines, routineLine, type TemplateDocument, type TemplateProfile } from "../core/templates";
import { t } from "../i18n";
import { AvatarDisc } from "./Avatar";
import { Section } from "./forms";
import { Pressable } from "./Pressable";
import { Symbol } from "./Symbol";
import { accentColor, Font, usePalette } from "./theme";

/// A piece of the template: its title over what it says, and its flags in a word or two (an
/// email address in orange, a removed key in gray). A row that can be picked toggles on a tap,
/// with a checkmark after it on iOS and Material's checkbox before it on Android.
export function TemplateItemRow({ title, detail, flags = [], titleLines = 1, leading, checked, onPress }: { title: string; detail?: string; flags?: readonly string[]; titleLines?: number; leading?: React.ReactNode; checked?: boolean; onPress?: () => void }) {
  const p = usePalette();
  const android = Platform.OS === "android";
  const flag = flagWords(flags);
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
  const selectable = checked !== undefined && !!onPress;
  return (
    <Pressable
      onPress={onPress}
      disabled={!selectable}
      style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]}
      accessibilityRole={selectable ? "checkbox" : undefined}
      accessibilityState={selectable ? { checked } : undefined}
    >
      {android && selectable ? (
        <ComposeHost matchContents>
          <Checkbox value={!!checked} onCheckedChange={onPress} />
        </ComposeHost>
      ) : null}
      {leading}
      <View style={styles.text}>
        <Text style={[styles.title, { color: p.label }]} numberOfLines={titleLines}>
          {title}
        </Text>
        {detail ? (
          <Text style={[styles.detail, { color: p.secondaryLabel }]} numberOfLines={2}>
            {detail}
          </Text>
        ) : null}
        {flag ? (
          <View style={styles.flag}>
            <Symbol name={flag.personal ? "exclamationmark.triangle.fill" : "key.fill"} size={12} color={flag.personal ? orange : p.tertiaryLabel} />
            <Text style={[styles.flagText, { color: flag.personal ? orange : p.tertiaryLabel }]} numberOfLines={1}>
              {flag.text}
            </Text>
          </View>
        ) : null}
      </View>
      {selectable && !android && checked ? <Symbol name="checkmark" size={16} color={p.tint} weight="semibold" /> : null}
    </Pressable>
  );
}

/// The profile, which every template holds: the look, the name, and what the bot is for.
export function TemplateProfileRow({ profile, flags }: { profile: TemplateProfile; flags?: readonly string[] }) {
  return (
    <TemplateItemRow
      title={profile.name}
      detail={profile.description}
      flags={flags}
      leading={<AvatarDisc symbol={profile.symbol_name || "sparkles"} accent={profile.accent || "indigo"} size={36} />}
    />
  );
}

/// What a template from a link or a file holds, as the sheet shows it before the bot is made:
/// the profile, skills, routines, and memories, none to pick.
export function TemplateDocumentSections({ template }: { template: TemplateDocument }) {
  const skills = template.skills ?? [];
  const routines = template.routines ?? [];
  const memories = template.memories ?? [];
  return (
    <>
      {template.profile ? (
        <Section title={t("Profile")}>
          <TemplateProfileRow profile={template.profile} />
        </Section>
      ) : null}
      {skills.length ? (
        <Section title={t("Skills")}>
          {skills.map((skill, index) => (
            <TemplateItemRow key={`skill-${index}`} title={skill.name} detail={skill.description} />
          ))}
        </Section>
      ) : null}
      {routines.length ? (
        <Section title={t("Routines")}>
          {routines.map((routine, index) => (
            <TemplateItemRow key={`routine-${index}`} title={routine.name} detail={routineLine(routine)} />
          ))}
        </Section>
      ) : null}
      {memories.length ? (
        <Section title={t("Memories")}>
          {memories.map((memory, index) => {
            const lines = memoryLines(memory);
            return <TemplateItemRow key={`memory-${index}`} title={lines.title} detail={lines.detail} titleLines={3} />;
          })}
        </Section>
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", minHeight: Platform.OS === "android" ? 56 : 44, paddingHorizontal: 16, paddingVertical: 10, gap: 12 },
  text: { flex: 1, gap: 2 },
  title: { fontSize: Font.body },
  detail: { fontSize: Font.small },
  flag: { flexDirection: "row", alignItems: "center", gap: 4, marginTop: 2 },
  flagText: { fontSize: Font.small },
});
