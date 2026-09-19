import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView } from "react-native";
import { engine } from "../src/core/engine";
import { MAX_GROUP_BOTS } from "../src/core/model";
import { useStore } from "../src/core/store";
import { t } from "../src/i18n";
import { BotAvatar } from "../src/ui/Avatar";
import { CheckRow, FieldRow, Section } from "../src/ui/forms";

export default function NewGroupScreen() {
  const router = useRouter();
  const bots = useStore((s) => s.bots);
  const [title, setTitle] = useState("");
  const [selected, setSelected] = useState<string[]>([]);

  function toggle(id: string) {
    setSelected((ids) => (ids.includes(id) ? ids.filter((i) => i !== id) : ids.length < MAX_GROUP_BOTS ? [...ids, id] : ids));
  }

  async function create() {
    try {
      const chat = await engine.createGroup(title, selected);
      router.dismiss();
      router.push(`/chat/${chat.id}`);
    } catch (error) {
      Alert.alert(t("Could not create the group"), error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: t("New Group Chat") }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button onPress={() => router.dismiss()}>{t("Cancel")}</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" disabled={selected.length === 0} onPress={() => void create()}>
          {t("Create")}
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section title={t("Name")}>
          <FieldRow value={title} onChangeText={setTitle} placeholder={t("Optional")} autoCapitalize="words" returnKeyType="done" />
        </Section>
        <Section title={t("Members · {count} of {max}", { count: selected.length, max: MAX_GROUP_BOTS })} footer={t("Members take turns after every message; the first one you pick owns the work.")}>
          {bots.map((bot) => (
            <CheckRow key={bot.id} title={bot.name} subtitle={bot.label} checked={selected.includes(bot.id)} onPress={() => toggle(bot.id)} leading={<BotAvatar bot={bot} size={36} />} />
          ))}
        </Section>
      </ScrollView>
    </>
  );
}
