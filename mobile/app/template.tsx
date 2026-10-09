// New Bot from Template, after the desktop apps' sheet: From takes a pasted link to a shared bot,
// which opens once it is whole, or a file from Choose File…; Open in Lorca on a link's page opens
// the sheet on its link (see +native-intent.tsx). What the template sets up shows once it opens:
// the Name, the Runner, the Provider, a row per plugin with the Runner's connections for it, then
// what the template holds, read-only. It previews again when the Runner, a connection, or the
// Runner's plugins change; a line says what is missing, else that routines start paused. Create
// makes the bot on its Runner and opens its chat.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import * as DocumentPicker from "expo-document-picker";
import { useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Platform, PlatformColor, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../src/core/engine";
import { connectedProviders, isRunner, providerLabel, PROVIDER_KINDS } from "../src/core/model";
import { pathOf } from "../src/core/prefs";
import { deviceIsOnline, useStore } from "../src/core/store";
import { connectionTitle, importNote, templateAddress, type TemplateImportPreview } from "../src/core/templates";
import { t, useLanguage } from "../src/i18n";
import { alert } from "../src/ui/alert";
import { AvatarDisc } from "../src/ui/Avatar";
import { deviceSymbol } from "../src/ui/devices";
import { CheckRow, FieldRow, Row, Section } from "../src/ui/forms";
import { FormToolbar } from "../src/ui/navigation";
import { Symbol } from "../src/ui/Symbol";
import { TemplateDocumentSections } from "../src/ui/templates";
import { accentColor, usePalette } from "../src/ui/theme";

type Source = { link: string } | { path: string };

export default function TemplateImportScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const params = useLocalSearchParams<{ link?: string }>();
  const devices = useStore((s) => s.devices);
  const runners = useMemo(() => devices.filter(isRunner), [devices]);
  const statuses = useStore((s) => s.providers);
  const connected = connectedProviders(statuses);
  const providers: readonly string[] = connected.length ? connected : PROVIDER_KINDS;
  const pendingLink = useStore((s) => s.pendingTemplateLink);
  const opened = params.link ? templateAddress(params.link) : undefined;

  const [from, setFrom] = useState(opened ?? "");
  const [source, setSource] = useState<Source | undefined>(opened ? { link: opened } : undefined);
  const [runnerId, setRunnerId] = useState<string>();
  const [provider, setProvider] = useState<string>(providers[0]);
  const [name, setName] = useState("");
  const nameEdited = useRef(false);
  /// The user's picks of a connection per plugin's service; the core picks the only ready one.
  const [mappings, setMappings] = useState<Record<string, string>>({});
  const [preview, setPreview] = useState<TemplateImportPreview>();
  const [failed, setFailed] = useState<string>();
  const [importing, setImporting] = useState(false);
  const generation = useRef(0);
  // The Runner picked, else the first online one, as New Bot picks it; the list may arrive after
  // the sheet, when a link opened it as the phone paired.
  const runner = runners.find((r) => r.id === runnerId) ?? runners.find((r) => deviceIsOnline(r.id)) ?? runners[0];
  const effectiveProvider = providers.includes(provider) ? provider : providers[0];
  // A plugin added or signed in on the Runner meanwhile changes what the import needs.
  const runnerPlugins = JSON.stringify(runner?.plugins ?? []);

  /// Another template: what it sets up shows again from the start.
  function open(next: Source) {
    if (importing || JSON.stringify(next) === JSON.stringify(source)) return;
    setSource(next);
    if (!nameEdited.current) setName("");
    setMappings({});
    setPreview(undefined);
    setFailed(undefined);
  }

  // A link opened with Open in Lorca while the sheet is up takes its place.
  useEffect(() => {
    if (!pendingLink || importing) return;
    useStore.setState({ pendingTemplateLink: null });
    setFrom(pendingLink);
    open({ link: pendingLink });
  }, [pendingLink]);

  useEffect(() => {
    if (!source) return;
    const current = ++generation.current;
    engine
      .previewTemplateImport({ ...source, runner_id: runner?.id, mappings })
      .then((next) => {
        if (current !== generation.current) return;
        setPreview(next);
        setFailed(undefined);
        if (!nameEdited.current && next.template?.profile?.name) setName((held) => held || next.template!.profile!.name);
      })
      .catch((error) => {
        if (current !== generation.current) return;
        setPreview(undefined);
        setFailed(error instanceof Error ? error.message : String(error));
      });
  }, [JSON.stringify(source), runner?.id, JSON.stringify(mappings), runnerPlugins]);

  /// A pasted link reads once it is whole: its page and the key after #.
  function changeFrom(text: string) {
    setFrom(text);
    const link = templateAddress(text);
    if (link) open({ link });
  }

  async function chooseFile() {
    if (importing) return;
    const picked = await DocumentPicker.getDocumentAsync({ copyToCacheDirectory: true, type: "*/*" });
    const file = picked.canceled ? undefined : picked.assets[0];
    if (!file) return;
    setFrom(file.name);
    open({ path: pathOf(file.uri) });
  }

  const template = preview?.template;
  const note = preview && template ? importNote(preview, runner?.name) : undefined;
  const canCreate = !!preview?.can_import && !!template && !!runner && !!name.trim() && !importing;

  async function create() {
    if (!canCreate || !source || !preview || !runner) return;
    setImporting(true);
    // The connection each plugin uses: the user's pick, else the one the core picked.
    const picked = Object.fromEntries(preview.requirements.filter((each) => each.selected).map((each) => [each.service_id, each.selected!]));
    try {
      const chatId = await engine.importTemplate({ ...source, runner_id: runner.id, name: name.trim(), provider: effectiveProvider, mappings: picked, expected_digest: preview.digest });
      router.dismiss();
      router.push(`/chat/${chatId}`);
    } catch (error) {
      setImporting(false);
      alert(t("Could not create the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
  const red = Platform.OS === "ios" ? PlatformColor("systemRed") : p.red;
  // What went wrong reading it: the core's error, or a file or link that holds no template.
  const problem = failed ?? (preview && !template ? preview.issues.join("\n") : undefined);

  return (
    <>
      <Stack.Screen options={{ title: t("New Bot") }} />
      <FormToolbar cancelLabel={t("Cancel")} saveLabel={t("Create")} saveDisabled={!canCreate} onCancel={() => router.dismiss()} onSave={() => void create()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section title={t("From")}>
          <FieldRow
            value={from}
            onChangeText={changeFrom}
            placeholder={t("Paste a link to a shared bot")}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            returnKeyType="done"
            editable={!importing}
            autoFocus={!opened}
          />
          <Row title={t("Choose File…")} icon="doc" onPress={importing ? undefined : () => void chooseFile()} />
        </Section>
        {problem ? <Text style={[styles.note, { color: red }]}>{problem}</Text> : null}
        {source && !preview && !failed ? (
          <View style={styles.loading}>
            <ActivityIndicator />
          </View>
        ) : null}

        {preview && template ? (
          <>
            <View style={styles.hero}>
              <AvatarDisc symbol={template.profile?.symbol_name || "sparkles"} accent={template.profile?.accent || "indigo"} size={76} />
            </View>
            <Section>
              <FieldRow
                label={t("Name")}
                value={name}
                onChangeText={(text) => {
                  nameEdited.current = true;
                  setName(text);
                }}
                placeholder={t("Name")}
                autoCapitalize="words"
                returnKeyType="done"
                editable={!importing}
              />
            </Section>
            <Section title={t("Runs on")} footer={runners.length ? undefined : t("Pair a computer running macOS, Linux, or Windows first. Phones never run bots.")}>
              {runners.map((r) => (
                <CheckRow
                  key={r.id}
                  title={r.name}
                  subtitle={`${r.model} · ${deviceIsOnline(r.id) ? t("Online") : t("Offline")}`}
                  checked={r.id === runner?.id}
                  onPress={() => {
                    if (importing || r.id === runner?.id) return;
                    setRunnerId(r.id);
                    setMappings({});
                  }}
                  leading={<Symbol name={deviceSymbol(r.os, r.model)} size={22} color={p.label} />}
                />
              ))}
            </Section>
            {runner ? (
              <Section title={t("Provider")} footer={connected.length ? undefined : t("No provider is connected yet; connect one in Settings before this bot answers.")}>
                {providers.map((kind) => (
                  <CheckRow key={kind} title={providerLabel(kind, statuses)} checked={kind === effectiveProvider} onPress={() => !importing && setProvider(kind)} />
                ))}
              </Section>
            ) : null}
            {runner && preview.requirements.length ? (
              <Section title={t("Plugins")}>
                {preview.requirements.map((plugin) => {
                  const selected = plugin.candidates.find((candidate) => candidate.id === plugin.selected);
                  return plugin.candidates.length ? (
                    <Row
                      key={plugin.service_id}
                      title={plugin.name}
                      menu={{
                        title: plugin.name,
                        value: selected ? connectionTitle(selected) : t("Choose…"),
                        choices: plugin.candidates.map((candidate) => ({
                          title: connectionTitle(candidate),
                          selected: candidate.id === plugin.selected,
                          onPress: () => !importing && setMappings((held) => ({ ...held, [plugin.service_id]: candidate.id })),
                        })),
                      }}
                    />
                  ) : (
                    <Row key={plugin.service_id} title={plugin.name} accessory={<Text style={[styles.missing, { color: orange }]}>{t("Not on {runner}", { runner: runner.name })}</Text>} />
                  );
                })}
              </Section>
            ) : null}
            {note ? <Text style={[styles.note, { color: note.warning ? orange : p.secondaryLabel }]}>{note.text}</Text> : null}
            <TemplateDocumentSections template={template} />
          </>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  hero: { alignItems: "center", paddingTop: 20 },
  loading: { paddingTop: 24, alignItems: "center" },
  note: { fontSize: 13, lineHeight: 18, marginHorizontal: 32, marginTop: 10 },
  missing: { fontSize: 15 },
});
