import { Stack, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { Alert, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { engine } from "../src/core/engine";
import type { CodexSelection } from "../src/core/model";
import { CodexSettings } from "../src/ui/CodexSettings";
import { connectedProviders, isRunner, providerLabel, PROVIDER_MODELS, THINKING_LEVELS, thinkingLabel } from "../src/core/model";
import { deviceIsOnline, useStore } from "../src/core/store";
import { t, useLanguage } from "../src/i18n";
import { CheckRow, FieldRow, Section } from "../src/ui/forms";
import { BOT_SYMBOLS, Symbol } from "../src/ui/Symbol";
import { ACCENTS, accentColors, usePalette } from "../src/ui/theme";
import { deviceSymbol } from "../src/ui/devices";
import { FormToolbar } from "../src/ui/navigation";

export default function NewBotScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const runners = useMemo(() => devices.filter(isRunner), [devices]);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [harness, setHarness] = useState<"lorca" | "codex">("lorca");
  const [codex, setCodex] = useState<CodexSelection>({ options: { speed: "default", approvals: "auto_review" } });
  const [symbol, setSymbol] = useState("sparkles");
  const [accent, setAccent] = useState("indigo");
  const [runnerId, setRunnerId] = useState<string>(() => runners.find((r) => deviceIsOnline(r.id))?.id ?? runners[0]?.id ?? "");
  const runner = runners.find((r) => r.id === runnerId);
  const connected = connectedProviders(useStore((s) => s.providers));
  const providers = connected.length ? connected : ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"];
  const [provider, setProvider] = useState<string>(providers[0]);
  const [model, setModel] = useState<string | undefined>(undefined);
  const [thinking, setThinking] = useState<string | undefined>(undefined);
  const effectiveProvider = providers.includes(provider) ? provider : providers[0];
  const canSave = name.trim().length > 0 && !!runnerId;

  async function save() {
    try {
      const { chatId } = await engine.createBot({ name, description, symbol_name: symbol, accent, runner_id: runnerId, provider: effectiveProvider, harness,
        codex_options: codex.options, model: harness === "codex" ? codex.model : model, thinking: harness === "codex" ? codex.thinking : thinking });
      router.dismiss();
      router.push(`/chat/${chatId}`);
    } catch (error) {
      Alert.alert(t("Could not create the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: t("New Bot") }} />
      <FormToolbar cancelLabel={t("Cancel")} saveLabel={t("Create")} saveDisabled={!canSave} onCancel={() => router.dismiss()} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <View style={styles.hero}>
          <LinearGradient colors={accentColors(accent, p.dark)} start={{ x: 0.3, y: 0 }} end={{ x: 0.7, y: 1 }} style={styles.heroAvatar}>
            <Symbol name={symbol} size={36} color="#FFFFFF" weight="semibold" />
          </LinearGradient>
        </View>
        <Section>
          <FieldRow label={t("Name")} value={name} onChangeText={setName} placeholder="Scout" autoFocus autoCapitalize="words" returnKeyType="next" />
        </Section>
        <Section title={t("Description")} footer={t("What this bot is for and how it should work. It also gets the team tools and the coding tools on its Runner.")}>
          <FieldRow value={description} onChangeText={setDescription} placeholder={t("Finds and summarizes sources")} multiline autoCapitalize="sentences" />
        </Section>
        <Section title={t("Look")}>
          <View style={styles.grid}>
            {BOT_SYMBOLS.map((s) => (
              <Pressable key={s} onPress={() => setSymbol(s)} style={[styles.symbolCell, { backgroundColor: s === symbol ? p.tint : p.fill }]} accessibilityLabel={s}>
                <Symbol name={s} size={20} color={s === symbol ? p.userBubbleText : p.label} />
              </Pressable>
            ))}
          </View>
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
        <Section title={t("Runtime")} footer={harness === "codex" ? t("Uses Codex's login, model, tools, and permissions on this Runner. Install Codex and run codex login there first.") : undefined}>
          <CheckRow title="Lorca" checked={harness === "lorca"} onPress={() => setHarness("lorca")} />
          <CheckRow title="Codex" checked={harness === "codex"} onPress={() => setHarness("codex")} />
        </Section>
        {runner && harness === "codex" && <Section title="Codex"><CodexSettings runnerId={runner.id} selection={codex} onChange={setCodex} /></Section>}
        {runner && harness === "lorca" && (
          <Section title={t("Provider")} footer={connected.length ? undefined : t("No provider is connected yet; connect one in Settings before this bot answers.")}>
            {providers.map((kind) => (
              <CheckRow key={kind} title={providerLabel(kind)} checked={kind === effectiveProvider} onPress={() => { setProvider(kind); setModel(undefined); setThinking(undefined); }} />
            ))}
          </Section>
        )}
        {runner && harness === "lorca" && PROVIDER_MODELS[effectiveProvider] && (
          <Section title={t("Model")}>
            <CheckRow title={t("Default")} subtitle={PROVIDER_MODELS[effectiveProvider][0].label} checked={!model} onPress={() => setModel(undefined)} />
            {PROVIDER_MODELS[effectiveProvider].map((m) => (
              <CheckRow key={m.id} title={m.label} subtitle={m.id} checked={model === m.id} onPress={() => setModel(m.id)} />
            ))}
          </Section>
        )}
        {runner && harness === "lorca" && THINKING_LEVELS[effectiveProvider] && (
          <Section title={t("Thinking")} footer={t("How much the model reasons before it answers. Higher levels are slower and cost more.")}>
            <CheckRow title={t("Default")} checked={!thinking} onPress={() => setThinking(undefined)} />
            {THINKING_LEVELS[effectiveProvider].map((level) => (
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
  heroAvatar: { width: 76, height: 76, borderRadius: 38, alignItems: "center", justifyContent: "center" },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 10, padding: 14 },
  symbolCell: { width: 40, height: 40, borderRadius: 10, alignItems: "center", justifyContent: "center" },
  swatchCell: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
  swatch: { width: 28, height: 28, borderRadius: 14, borderWidth: 2.5 },
});
