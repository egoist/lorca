// A bot's Access, slid in from its Access row in Details, or opened from a "needs more access"
// card: the plugins on its Runner, each with how far the bot may use it and a screen of its own
// for that and its tools, whether its messages wait as drafts, then Files and Shell commands under
// the Runner's name. Each change saves at once. The desktop apps' Access sheet holds the same.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect } from "react";
import { ScrollView, StyleSheet } from "react-native";
import {
  chosenTools,
  draftsOn,
  filesLevel,
  levelTitle,
  loadAccessCatalog,
  LEVELS,
  pluginLevel,
  reaches,
  setBotPermissions,
  shellOn,
  toolDoes,
  toolsSummary,
  useAccessStore,
  withPlugin,
  withTool,
  type AccessPlugin,
  type FilesLevel,
} from "../core/access";
import type { BotPermissions } from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { CheckRow, Row, Section, ToggleRow } from "./forms";
import { CloseToolbar } from "./navigation";

/// The bot, its Runner, and the plugins to show: the Runner's answer once it has one, its own
/// list of plugins until then.
function useAccess(botId: string) {
  const bot = useStore((s) => s.bots.find((each) => each.id === botId));
  const runner = useStore((s) => s.devices.find((device) => device.id === bot?.runner_id));
  const catalog = useAccessStore((s) => s.byBot[botId]);
  const plugins: AccessPlugin[] = Array.isArray(catalog) ? catalog : (runner?.plugins ?? []).map((plugin) => ({ id: plugin.id, name: plugin.name, tools: [] }));
  const save = (policy: BotPermissions) =>
    setBotPermissions(botId, policy).catch((error) => alert(t("Could not update the bot"), error instanceof Error ? error.message : String(error)));
  return { bot, runner, plugins, failed: catalog === "failed", save };
}

export default function AccessScreen() {
  useLanguage();
  const { id, close } = useLocalSearchParams<{ id: string; close?: string }>();
  const router = useRouter();
  const { bot, runner, plugins, failed, save } = useAccess(id);
  // What each plugin offers is the Runner's to say; asked whenever the screen opens.
  useEffect(() => {
    void loadAccessCatalog(id);
  }, [id]);
  if (!bot) return null;
  const policy = bot.permissions;
  const runnerName = runner?.name ?? t("its Runner");
  const icons = new Map((runner?.plugins ?? []).map((plugin) => [plugin.id, plugin.icon]));

  return (
    <>
      <Stack.Screen options={{ title: t("Access") }} />
      {/* Opened from a card, it is the sheet's first screen and closes it. */}
      {close ? <CloseToolbar label={t("Done")} onClose={() => router.back()} /> : null}
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        {plugins.length > 0 && (
          <Section title={t("Plugins")} footer={failed ? t("Couldn't get the tools from {runner}.", { runner: runnerName }) : undefined}>
            {plugins.map((plugin) => (
              <Row
                key={plugin.id}
                title={plugin.name}
                subtitle={toolsSummary(policy, plugin)}
                detail={levelTitle(pluginLevel(policy, plugin.id))}
                icon={icons.get(plugin.id) || "puzzlepiece.extension"}
                chevron
                onPress={() => router.push({ pathname: "/chat-info/access-plugin", params: { bot: bot.id, plugin: plugin.id } })}
              />
            ))}
          </Section>
        )}
        <Section title={t("Messages")} footer={t("Emails and Slack messages wait in the chat for you to send. Off, {name} sends them itself where the service can.", { name: bot.name })}>
          <ToggleRow title={t("Draft first")} value={draftsOn(policy)} onValueChange={(on) => void save({ ...policy, filesystem: filesLevel(policy), shell: shellOn(policy), drafts: on })} />
        </Section>
        {/* Files and shell commands are the Runner's, so its section goes by the Runner's name. */}
        <Section title={runner?.name ?? t("Runner")} footer={t("Shell commands run as you on {runner} and can reach anything you can there.", { runner: runnerName })}>
          <Row
            title={t("Files")}
            menu={{
              value: levelTitle(filesLevel(policy)),
              title: t("Files"),
              choices: (["write", "read", "none"] as FilesLevel[]).map((level) => ({
                title: levelTitle(level),
                selected: filesLevel(policy) === level,
                onPress: () => void save({ ...policy, filesystem: level, shell: shellOn(policy) }),
              })),
            }}
          />
          <ToggleRow title={t("Shell commands")} value={shellOn(policy)} onValueChange={(on) => void save({ ...policy, filesystem: filesLevel(policy), shell: on })} />
        </Section>
      </ScrollView>
    </>
  );
}

/// One plugin's Access, slid in from its row: how far the bot may use it, then its tools, each a
/// checkbox with what it does. A tool beyond the level stays off and dimmed until the level
/// reaches it; Read and draft shows only for a plugin with a tool that drafts.
export function AccessPluginScreen() {
  useLanguage();
  const { bot: botId, plugin: pluginId } = useLocalSearchParams<{ bot: string; plugin: string }>();
  const { bot, plugins, save } = useAccess(botId);
  const plugin = plugins.find((each) => each.id === pluginId);
  if (!bot || !plugin) return null;
  const policy = bot.permissions;
  const level = pluginLevel(policy, plugin.id);
  const drafts = level === "draft" || plugin.tools.some((tool) => tool.capability === "draft");
  const chosen = chosenTools(policy, plugin.id);

  return (
    <>
      <Stack.Screen options={{ title: plugin.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section>
          {LEVELS.filter((each) => each !== "draft" || drafts).map((each) => (
            <CheckRow key={each} title={levelTitle(each)} checked={level === each} onPress={() => void save(withPlugin(policy, plugins, plugin.id, each, chosen))} />
          ))}
        </Section>
        {plugin.tools.length > 0 && level !== "none" && (
          <Section title={t("Tools")}>
            {plugin.tools.map((tool) => {
              const reached = reaches(level, tool.capability);
              const on = reached && (!chosen || chosen.includes(tool.name));
              return (
                <CheckRow
                  key={tool.name}
                  multiple
                  title={tool.title || tool.name}
                  subtitle={toolDoes(tool.capability)}
                  checked={on}
                  disabled={!reached}
                  onPress={() => void save(withTool(policy, plugins, plugin, tool.name, !on))}
                />
              );
            })}
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
});
