import { Stack, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { Alert, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { engine } from "../src/core/engine";
import { isRunner, providerLabel } from "../src/core/model";
import { deviceIsOnline, useStore } from "../src/core/store";
import { CheckRow, FieldRow, Section } from "../src/ui/forms";
import { BOT_SYMBOLS, Symbol } from "../src/ui/Symbol";
import { ACCENTS, accentColors, usePalette } from "../src/ui/theme";
import { deviceSymbol } from "../src/ui/devices";

const MODELS: Record<string, { id: string; label: string }[]> = {
  deepseek: [
    { id: "deepseek-flash", label: "V4.1 Flash" },
    { id: "deepseek-v4-pro", label: "V4 Pro (reasoning)" },
  ],
  chatgpt: [
    { id: "gpt-5.6-terra", label: "GPT-5.6 Terra" },
    { id: "gpt-6-astra", label: "GPT-6 Astra" },
    { id: "gpt-5.6-sol", label: "GPT-5.6 Sol" },
    { id: "gpt-5.6-luna", label: "GPT-5.6 Luna" },
    { id: "gpt-5.5", label: "GPT-5.5" },
  ],
};

export default function NewBotScreen() {
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const runners = useMemo(() => devices.filter(isRunner), [devices]);
  const [name, setName] = useState("");
  const [label, setLabel] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [symbol, setSymbol] = useState("sparkles");
  const [accent, setAccent] = useState("indigo");
  const [runnerId, setRunnerId] = useState<string>(() => runners.find((r) => deviceIsOnline(r.id))?.id ?? runners[0]?.id ?? "");
  const runner = runners.find((r) => r.id === runnerId);
  const providers = runner?.providers_connected.length ? runner.providers_connected : ["deepseek", "chatgpt"];
  const [provider, setProvider] = useState<string>(providers[0]);
  const [model, setModel] = useState<string | undefined>(undefined);
  const effectiveProvider = providers.includes(provider) ? provider : providers[0];
  const canSave = name.trim().length > 0 && !!runnerId;

  function save() {
    try {
      const { chat } = engine.createBot({ name, label, description, instructions, symbol_name: symbol, accent, runner_id: runnerId, provider: effectiveProvider, model });
      router.dismiss();
      router.push(`/chat/${chat.id}`);
    } catch (error) {
      Alert.alert("Could not create the bot", error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: "New Bot" }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button onPress={() => router.dismiss()}>Cancel</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" disabled={!canSave} onPress={save}>
          Create
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <View style={styles.hero}>
          <LinearGradient colors={accentColors(accent, p.dark)} start={{ x: 0.3, y: 0 }} end={{ x: 0.7, y: 1 }} style={styles.heroAvatar}>
            <Symbol name={symbol} size={36} color="#FFFFFF" weight="semibold" />
          </LinearGradient>
        </View>
        <Section>
          <FieldRow label="Name" value={name} onChangeText={setName} placeholder="Scout" autoFocus autoCapitalize="words" returnKeyType="next" />
          <FieldRow label="Label" value={label} onChangeText={setLabel} placeholder="Research" autoCapitalize="sentences" />
          <FieldRow label="Description" value={description} onChangeText={setDescription} placeholder="Finds and summarizes sources" autoCapitalize="sentences" />
        </Section>
        <Section title="Instructions" footer="What this bot is for and how it should work. It also gets the team tools and the coding tools on its Runner.">
          <FieldRow value={instructions} onChangeText={setInstructions} placeholder="You are…" multiline autoCapitalize="sentences" />
        </Section>
        <Section title="Look">
          <View style={styles.grid}>
            {BOT_SYMBOLS.map((s) => (
              <Pressable key={s} onPress={() => setSymbol(s)} style={[styles.symbolCell, { backgroundColor: s === symbol ? p.tint : p.fill }]} accessibilityLabel={s}>
                <Symbol name={s} size={20} color={s === symbol ? "#FFFFFF" : p.label} />
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
        <Section title="Runs on" footer={runners.length ? "Bots run on a paired desktop Device with the CLI; the provider's credentials stay there." : "Pair a Mac, Linux, or Windows machine first. Phones never run bots."}>
          {runners.map((r) => (
            <CheckRow
              key={r.id}
              title={r.name}
              subtitle={`${r.model} · ${deviceIsOnline(r.id) ? "Online" : "Offline"}`}
              checked={r.id === runnerId}
              onPress={() => setRunnerId(r.id)}
              leading={<Symbol name={deviceSymbol(r.os, r.model)} size={22} color={p.label} />}
            />
          ))}
        </Section>
        {runner && (
          <Section title="Provider" footer={runner.providers_connected.length ? undefined : `${runner.name} has no provider connected yet; connect one there before this bot answers.`}>
            {providers.map((kind) => (
              <CheckRow key={kind} title={providerLabel(kind)} checked={kind === effectiveProvider} onPress={() => { setProvider(kind); setModel(undefined); }} />
            ))}
          </Section>
        )}
        {runner && MODELS[effectiveProvider] && (
          <Section title="Model">
            <CheckRow title="Default" subtitle={MODELS[effectiveProvider][0].label} checked={!model} onPress={() => setModel(undefined)} />
            {MODELS[effectiveProvider].slice(1).map((m) => (
              <CheckRow key={m.id} title={m.label} subtitle={m.id} checked={model === m.id} onPress={() => setModel(m.id)} />
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
