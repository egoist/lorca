import { Stack, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { Pressable } from "../src/ui/Pressable";
import { engine } from "../src/core/engine";
import { connectedProviders, isRunner, providerLabel, providerModels, PROVIDER_KINDS, thinkingLabel, thinkingLevels, withCustomModels } from "../src/core/model";
import { deviceIsOnline, useStore } from "../src/core/store";
import { t, useLanguage } from "../src/i18n";
import { CheckRow, FieldRow, Section } from "../src/ui/forms";
import { Symbol } from "../src/ui/Symbol";
import { LookAvatar, ShapePicker } from "../src/ui/Avatar";
import type { BotShape } from "../src/ui/characters";
import { ACCENTS, accentColors, usePalette } from "../src/ui/theme";
import { deviceSymbol } from "../src/ui/devices";
import { FormToolbar } from "../src/ui/navigation";
import { alert } from "../src/ui/alert";

export default function NewBotScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const runners = useMemo(() => devices.filter(isRunner), [devices]);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [shape, setShape] = useState<BotShape>("blob");
  const [accent, setAccent] = useState("indigo");
  const [runnerId, setRunnerId] = useState<string>(() => runners.find((r) => deviceIsOnline(r.id))?.id ?? runners[0]?.id ?? "");
  const runner = runners.find((r) => r.id === runnerId);
  const statuses = useStore((s) => s.providers);
  // The connected providers a bot on the Runner can run with, custom ones after the built-ins,
  // or every built-in while none is. Decision providers and models are Auto-review's alone.
  const connected = connectedProviders(statuses, runnerId);
  const catalog = withCustomModels(useStore((s) => s.models), statuses);
  const providers: readonly string[] = connected.length ? connected : PROVIDER_KINDS;
  const [provider, setProvider] = useState<string>(providers[0]);
  const [model, setModel] = useState<string | undefined>(undefined);
  const [thinking, setThinking] = useState<string | undefined>(undefined);
  const effectiveProvider = providers.includes(provider) ? provider : providers[0];
  const offered = providerModels(catalog, effectiveProvider);
  const levels = thinkingLevels(catalog, effectiveProvider, model);
  // Switching models keeps the thinking level only where the new model takes it.
  function pickModel(next: string | undefined) {
    setModel(next);
    if (thinking && !thinkingLevels(catalog, effectiveProvider, next).includes(thinking)) setThinking(undefined);
  }
  const canSave = name.trim().length > 0 && !!runnerId;

  async function save() {
    try {
      const { chatId } = await engine.createBot({ name, description, shape, accent, runner_id: runnerId, provider: effectiveProvider, model, thinking });
      router.dismiss();
      router.push(`/chat/${chatId}`);
    } catch (error) {
      alert(t("Could not create the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: t("New Bot") }} />
      <FormToolbar cancelLabel={t("Cancel")} saveLabel={t("Create")} saveDisabled={!canSave} onCancel={() => router.dismiss()} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <View style={styles.hero}>
          <LookAvatar shape={shape} accent={accent} size={76} />
        </View>
        <Section>
          <FieldRow label={t("Name")} value={name} onChangeText={setName} placeholder="Scout" autoFocus autoCapitalize="words" returnKeyType="next" />
        </Section>
        <Section title={t("Description")} footer={t("What this bot is for and how it should work. It also gets the team tools and the coding tools on its Runner.")}>
          <FieldRow value={description} onChangeText={setDescription} placeholder={t("Finds and summarizes sources")} multiline autoCapitalize="sentences" />
        </Section>
        <Section title={t("Look")}>
          <ShapePicker shape={shape} accent={accent} onPick={setShape} />
          <View style={styles.grid}>
            {(Object.keys(ACCENTS) as (keyof typeof ACCENTS)[]).map((a) => (
              <Pressable key={a} onPress={() => setAccent(a)} style={styles.swatchCell} accessibilityLabel={a}>
                <View style={[styles.swatch, { backgroundColor: accentColors(a, p.dark)[1], borderColor: a === accent ? p.label : "transparent" }]} />
              </Pressable>
            ))}
          </View>
        </Section>
        <Section title={t("Runs on")} footer={runners.length ? t("Bots run on a paired desktop Device with the CLI, using your account's provider credentials.") : t("Pair a computer running macOS, Linux, or Windows first. Phones never run bots.")}>
          {runners.map((r) => (
            <CheckRow
              key={r.id}
              title={r.name}
              subtitle={`${r.model} · ${deviceIsOnline(r.id) ? t("Online") : t("Offline")}`}
              checked={r.id === runnerId}
              onPress={() => setRunnerId(r.id)}
              leading={<Symbol name={deviceSymbol(r.os, r.model)} size={22} color={p.label} />}
            />
          ))}
        </Section>
        {runner && (
          <Section title={t("Provider")} footer={connected.length ? undefined : t("No provider is connected yet; connect one in Settings before this bot answers.")}>
            {providers.map((kind) => (
              <CheckRow key={kind} title={providerLabel(kind, statuses)} checked={kind === effectiveProvider} onPress={() => { setProvider(kind); setModel(undefined); setThinking(undefined); }} />
            ))}
          </Section>
        )}
        {runner && offered.length > 0 && (
          <Section title={t("Model")}>
            <CheckRow title={t("Default")} subtitle={offered[0].name} checked={!model} onPress={() => pickModel(undefined)} />
            {offered.map((m) => (
              <CheckRow key={m.id} title={m.name} subtitle={m.id} checked={model === m.id} onPress={() => pickModel(m.id)} />
            ))}
          </Section>
        )}
        {/* Only the levels this model takes; a model without any has no choice to make. */}
        {runner && levels.length > 0 && (
          <Section title={t("Thinking")} footer={t("How much the model reasons before it answers. Higher levels are slower and cost more.")}>
            <CheckRow title={t("Default")} checked={!thinking} onPress={() => setThinking(undefined)} />
            {levels.map((level) => (
              <CheckRow key={level} title={thinkingLabel(level)} checked={thinking === level} onPress={() => setThinking(level)} />
            ))}
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  hero: { alignItems: "center", paddingTop: 8 },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 10, padding: 14 },
  swatchCell: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
  swatch: { width: 28, height: 28, borderRadius: 14, borderWidth: 2.5 },
});
