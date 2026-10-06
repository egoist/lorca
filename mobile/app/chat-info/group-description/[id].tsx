// What a group is for, pushed from its row in Group Info. Saving puts it in the encrypted roster,
// so every paired Device and every member's next turn see it.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView, StyleSheet } from "react-native";
import { engine } from "../../../src/core/engine";
import { useChat } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { FieldRow, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";

export default function GroupDescriptionScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const chat = useChat(id);
  const [description, setDescription] = useState(chat?.description ?? "");

  if (!chat) return null;

  function save() {
    engine.setGroupDescription(chat!.id, description);
    router.back();
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Description") }} />
      <SaveToolbar label={t("Done")} onSave={save} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section footer={t("What this group is for. Every bot in it reads this.")}>
          <FieldRow value={description} onChangeText={setDescription} placeholder={t("Plans the launch and keeps the checklist current")} multiline autoFocus autoCapitalize="sentences" style={styles.editor} />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  editor: { minHeight: 220 },
});
