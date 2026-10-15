// Settings › Coding Agents: the agents bots hand coding work to on their Runner, in the account's
// order of preference, each on or off. A row's grip drags it to another place; the switch turns it
// on or off. Each change sends the whole list (`coding_agents.set`), which syncs to every Device;
// the list shows it at once and goes back, with an alert, when the core refuses. A phone cannot
// tell which agents a Runner has installed, so the rows don't say.

import { Stack } from "expo-router";
import { Host as ComposeHost, Switch as ComposeSwitch } from "@expo/ui/jetpack-compose";
import { Platform, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { agentName, type CodingAgents } from "../../src/core/model";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { AgentLogo } from "../../src/ui/AgentLogo";
import { alert } from "../../src/ui/alert";
import { Section } from "../../src/ui/forms";
import { ReorderableRows } from "../../src/ui/ReorderableRows";
import { Font, usePalette } from "../../src/ui/theme";

const ROW_HEIGHT = Platform.OS === "android" ? 56 : 44;

export default function CodingAgentsScreen() {
  useLanguage();
  const p = usePalette();
  const settings = useStore((s) => s.coding_agents);
  const agents = settings.agents;

  const save = (next: CodingAgents) => {
    engine.setCodingAgents(next).catch((error) => alert(t("Couldn't save the coding agents"), error instanceof Error ? error.message : String(error)));
  };
  const reorder = (kinds: string[]) => save({ agents: kinds.map((kind) => agents.find((choice) => choice.kind === kind)!).filter(Boolean) });
  const toggle = (kind: string, enabled: boolean) => save({ agents: agents.map((choice) => (choice.kind === kind ? { ...choice, enabled } : choice)) });

  return (
    <>
      <Stack.Screen options={{ title: t("Coding Agents") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          footer={`${t("Bots hand coding work to these agents on their Runner. A bot uses the first one that's on and installed there, unless you asked for another.")} ${t("Drag an agent by its handle to change the order.")}`}
        >
          <ReorderableRows
            keys={agents.map((choice) => choice.kind)}
            height={ROW_HEIGHT}
            label={agentName}
            onMove={reorder}
            row={(kind) => {
              const choice = agents.find((agent) => agent.kind === kind)!;
              const name = agentName(kind);
              return (
                <View style={styles.row}>
                  <AgentLogo kind={kind} size={20} />
                  <Text style={[styles.name, { color: p.label }]} numberOfLines={1}>
                    {name}
                  </Text>
                  {Platform.OS === "android" ? (
                    <ComposeHost matchContents>
                      <ComposeSwitch value={choice.enabled} onCheckedChange={(on) => toggle(kind, on)} />
                    </ComposeHost>
                  ) : (
                    <Switch value={choice.enabled} onValueChange={(on) => toggle(kind, on)} accessibilityLabel={name} />
                  )}
                </View>
              );
            }}
          />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  row: { flex: 1, flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 16 },
  name: { fontSize: Font.body, flex: 1 },
});
