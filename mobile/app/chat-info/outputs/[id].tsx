// Every output of a chat, for its details' View All: one row per output, the latest first.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { ScrollView } from "react-native";
import { useChat, useOutputs } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { Section } from "../../../src/ui/forms";
import { OutputRow } from "../../../src/ui/outputs";

export default function OutputsScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const chat = useChat(id);
  const outputs = useOutputs(id);
  return (
    <>
      <Stack.Screen options={{ title: t("Outputs") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section>
          {outputs.map((series) => (
            <OutputRow key={series.id} series={series} isGroup={chat?.kind === "group"} onPress={() => router.push({ pathname: "/chat-info/output/[id]", params: { id: series.id, chat: id } })} />
          ))}
        </Section>
      </ScrollView>
    </>
  );
}
