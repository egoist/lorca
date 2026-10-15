// Settings › Coding Agents: the agents bots hand coding work to on their Runner, in the account's
// order of preference, each on or off. A tap on an agent drops a menu that moves it up or down; the
// switch turns it on or off. Each change sends the whole list (`coding_agents.set`), which syncs to
// every Device; the list shows it at once and goes back, with an alert, when the core refuses. A
// phone cannot tell which agents a Runner has installed, so the rows don't say.

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
import { RowMenu } from "../../src/ui/RowMenu";
import { Font, usePalette } from "../../src/ui/theme";

export default function CodingAgentsScreen() {
  useLanguage();
  const p = usePalette();
  const settings = useStore((s) => s.coding_agents);
  const agents = settings.agents;

  const save = (next: CodingAgents) => {
    engine.setCodingAgents(next).catch((error) => alert(t("Couldn't save the coding agents"), error instanceof Error ? error.message : String(error)));
  };
  const move = (from: number, to: number) => {
    if (to < 0 || to >= agents.length) return;
    const next = [...agents];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    save({ agents: next });
  };
  const toggle = (index: number, enabled: boolean) => save({ agents: agents.map((choice, at) => (at === index ? { ...choice, enabled } : choice)) });

  return (
    <>
      <Stack.Screen options={{ title: t("Coding Agents") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          footer={`${t("Bots hand coding work to these agents on their Runner. A bot uses the first one that's on and installed there, unless you asked for another.")} ${t("Tap an agent to move it up or down.")}`}
        >
          {agents.map((choice, index) => {
            const name = agentName(choice.kind);
            return (
              <View key={choice.kind} style={styles.row}>
                <View style={styles.grow}>
                  <RowMenu
                    label={name}
                    hint={t("Moves it up or down the list")}
                    actions={[
                      { id: "up", title: t("Move Up"), symbol: "arrow.up", disabled: index === 0 },
                      { id: "down", title: t("Move Down"), symbol: "arrow.down", disabled: index === agents.length - 1 },
                    ]}
                    onChoose={(action) => move(index, action === "up" ? index - 1 : index + 1)}
                  >
                    <View style={styles.label}>
                      <AgentLogo kind={choice.kind} size={20} color={choice.enabled ? p.label : p.tertiaryLabel} />
                      <Text style={[styles.name, { color: p.label }]} numberOfLines={1}>
                        {name}
                      </Text>
                    </View>
                  </RowMenu>
                </View>
                {Platform.OS === "android" ? (
                  <ComposeHost matchContents>
                    <ComposeSwitch value={choice.enabled} onCheckedChange={(on) => toggle(index, on)} />
                  </ComposeHost>
                ) : (
                  <Switch value={choice.enabled} onValueChange={(on) => toggle(index, on)} accessibilityLabel={name} />
                )}
              </View>
            );
          })}
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", paddingRight: 16, gap: 12 },
  grow: { flex: 1 },
  label: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: Platform.OS === "android" ? 56 : 44, paddingLeft: 16 },
  name: { fontSize: Font.body, flexShrink: 1 },
});
