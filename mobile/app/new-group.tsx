import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView } from "react-native";
import { engine } from "../src/core/engine";
import { MAX_GROUP_BOTS } from "../src/core/model";
import { useStore } from "../src/core/store";
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

  function create() {
    try {
      const chat = engine.createGroup(title, selected);
      router.dismiss();
      router.push(`/chat/${chat.id}`);
    } catch (error) {
      Alert.alert("Could not create the group", error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: "New Group Chat" }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button onPress={() => router.dismiss()}>Cancel</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" disabled={selected.length === 0} onPress={create}>
          Create
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section title="Name">
          <FieldRow value={title} onChangeText={setTitle} placeholder="Optional" autoCapitalize="words" returnKeyType="done" />
        </Section>
        <Section title={`Members · ${selected.length} of ${MAX_GROUP_BOTS}`} footer="Members take turns after every message; the first one you pick owns the work.">
          {bots.map((bot) => (
            <CheckRow key={bot.id} title={bot.name} subtitle={bot.tagline} checked={selected.includes(bot.id)} onPress={() => toggle(bot.id)} leading={<BotAvatar bot={bot} size={36} />} />
          ))}
        </Section>
      </ScrollView>
    </>
  );
}
