// The bot's full behavioral description, pushed from its compact row in Details. Saving updates
// the encrypted profile in the roster, so every paired Device and the bot's next turn see it.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView, StyleSheet } from "react-native";
import { engine } from "../../../src/core/engine";
import { useBotMap } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { FieldRow, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";

export default function BotDescriptionScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const bot = useBotMap().get(id);
  const [description, setDescription] = useState(bot?.description ?? "");
  const [saving, setSaving] = useState(false);
  const value = description.trim();
  const changed = !!bot && value !== bot.description.trim();

  if (!bot) return null;

  async function save() {
    if (!bot || saving) return;
    if (!changed) {
      router.back();
      return;
    }
    setSaving(true);
    try {
      await engine.updateBot(bot.id, { description: value });
      router.back();
    } catch (error) {
      setSaving(false);
      Alert.alert(t("Could not update the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Description") }} />
      <SaveToolbar label={t("Done")} disabled={saving} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section footer={t("What this bot is for and how it should work. It also gets the team tools and the coding tools on its Runner.")}>
          <FieldRow value={description} onChangeText={setDescription} placeholder={t("Finds and summarizes sources")} multiline autoFocus autoCapitalize="sentences" style={styles.editor} />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  editor: { minHeight: 220 },
});
