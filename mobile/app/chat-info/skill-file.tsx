// A reference or a script of the skill open behind it: its name in its folder and its text. The
// file is text the skill holds; nothing here reads or writes a file on the phone, and a script
// runs only when a bot runs it, with the usual Auto-review. Edits land in the skill's form and are
// saved with the skill.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Platform, ScrollView, StyleSheet } from "react-native";
import { t, useLanguage } from "../../src/i18n";
import { FieldRow, Row, Section } from "../../src/ui/forms";
import { safeFileName, useSkillForm, type SkillFile } from "../../src/ui/skills";

export default function SkillFileScreen() {
  useLanguage();
  const { key } = useLocalSearchParams<{ key: string }>();
  const router = useRouter();
  const files = useSkillForm((s) => s.files);
  const file = files.find((each) => String(each.key) === key);
  if (!file) return null;

  const update = (change: Partial<SkillFile>) => useSkillForm.setState((s) => ({ files: s.files.map((each) => (each.key === file.key ? { ...each, ...change } : each)) }));
  const taken = files.some((other) => other !== file && other.folder === file.folder && other.name === file.name);
  const problem = !safeFileName(file.name) ? t("Use letters, numbers, dots, hyphens, and underscores.") : taken ? t("Another file here has this name.") : undefined;

  return (
    <>
      <Stack.Screen options={{ title: file.folder === "scripts" ? t("Script") : t("Reference") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section footer={problem}>
          <FieldRow label={t("Name")} value={file.name} onChangeText={(name) => update({ name })} autoCapitalize="none" autoCorrect={false} style={styles.mono} />
        </Section>
        <Section>
          <FieldRow
            value={file.text}
            onChangeText={(text) => update({ text })}
            placeholder={file.folder === "scripts" ? t("Script text") : t("Reference text")}
            multiline
            autoCapitalize="none"
            autoCorrect={false}
            style={[styles.mono, styles.text]}
          />
        </Section>
        <Section>
          <Row
            title={t("Remove")}
            icon="trash"
            destructive
            onPress={() => {
              useSkillForm.setState((s) => ({ files: s.files.filter((each) => each.key !== file.key) }));
              router.back();
            }}
          />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  mono: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 14 },
  text: { minHeight: 220 },
});
