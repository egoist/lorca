// A custom provider, slid in from Settings inside the same sheet: any server that speaks
// OpenAI's or Anthropic's API, such as a gateway or a model server on the user's network. With a
// `preset` the form starts from a server people often add; with a provider's `kind` it edits that
// one; with neither it starts empty. The core lists the server's models as the URL and the key
// change, and reaches the server again before it saves; the provider joins the account's
// encrypted credentials, which every paired Device shares.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import {
  CUSTOM_APIS,
  customAPI,
  customPreset,
  customRequestURL,
  defaultModelId,
  defaultProviderName,
  isCustomProvider,
  isHTTPURL,
  isLoopbackHost,
  modelLabel,
  modelListingNote,
  presetForURL,
  savedModelRows,
  selectedModelIds,
  urlHost,
  type CustomAPI,
} from "../../src/core/model";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { FieldRow, Row, Section } from "../../src/ui/forms";
import { chooseDefaultModel, setModelListing, startModelDraft, takeModelListing, useModelDraft } from "../../src/ui/modelDraft";
import { usePalette } from "../../src/ui/theme";
import { alert } from "../../src/ui/alert";

/// How long the form waits after the URL, protocol, or key last changed before it asks the server.
const LISTING_DELAY_MS = 500;

function param(value: string | string[] | undefined): string | undefined {
  return (Array.isArray(value) ? value[0] : value) || undefined;
}

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export default function CustomProviderScreen() {
  useLanguage();
  const params = useLocalSearchParams<{ kind?: string; preset?: string }>();
  const kind = param(params.kind);
  const router = useRouter();
  const p = usePalette();
  const status = useStore((s) => (kind ? s.providers.find((provider) => provider.kind === kind) : undefined));
  // The provider as it stood when the form opened. The fields start from it, and the form stays
  // as it is while a delete started here takes the provider out of the store.
  const [saved] = useState(status);
  const adding = kind ? undefined : customPreset(param(params.preset));
  const [name, setName] = useState(saved?.name ?? adding?.name ?? "");
  const [api, setAPI] = useState<CustomAPI>(customAPI(saved?.api ?? adding?.api).id);
  const [baseURL, setBaseURL] = useState(saved?.base_url ?? adding?.baseURL ?? "");
  const [apiKey, setAPIKey] = useState("");
  // An edited provider's key comes from the core before the server is first asked for models.
  const [keyLoaded, setKeyLoaded] = useState(!saved);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [initialRows] = useState(() => savedModelRows(saved?.models ?? []));
  const rows = useModelDraft((s) => s.rows);
  const chosenDefault = useModelDraft((s) => s.chosenDefault);
  const listing = useModelDraft((s) => s.listing);
  /// The latest ask for the server's models; an answer to an older one is dropped.
  const asked = useRef(0);
  /// Past the first ask, which goes at once: the later ones wait for the typing to pause.
  const opened = useRef(false);

  // The picker this form pushes works on the same rows.
  const [startsWithURL] = useState(() => isHTTPURL(baseURL));
  useLayoutEffect(() => startModelDraft(initialRows, startsWithURL), [initialRows, startsWithURL]);

  // A status carries only a masked key: the form asks for the saved one, and keeps what the user
  // typed meanwhile.
  useEffect(() => {
    if (!saved) return;
    let current = true;
    engine
      .providerAPIKey(saved.kind)
      .then(({ api_key }) => {
        if (current && api_key) setAPIKey((typed) => typed || api_key);
      })
      .catch(() => {})
      .finally(() => {
        if (current) setKeyLoaded(true);
      });
    return () => {
      current = false;
    };
  }, [saved]);

  // The server's models: at once when the form opens with a URL, then a moment after the URL,
  // the protocol, or the key last changed.
  useEffect(() => () => void (asked.current += 1), []);
  useEffect(() => {
    if (!keyLoaded) return;
    const url = baseURL.trim();
    const ask = ++asked.current;
    const delay = opened.current ? LISTING_DELAY_MS : 0;
    opened.current = true;
    if (!isHTTPURL(url)) return setModelListing({ state: "none" });
    setModelListing({ state: "loading" });
    const timer = setTimeout(() => {
      engine
        .listCustomModels({ name: name.trim() || defaultProviderName(url), api, baseURL: url, apiKey })
        .then(({ listed, models }) => {
          if (ask === asked.current) takeModelListing(listed, models);
        })
        .catch((cause) => {
          if (ask === asked.current) setModelListing({ state: "error", message: messageOf(cause) });
        });
    }, delay);
    return () => clearTimeout(timer);
  }, [api, baseURL, apiKey, keyLoaded]);

  if (kind && (!saved || !isCustomProvider(kind))) {
    return (
      <>
        <Stack.Screen options={{ title: t("Provider") }} />
        <View style={styles.center}>
          <Text style={{ color: p.secondaryLabel }}>{t("Unknown provider")}</Text>
        </View>
      </>
    );
  }

  const protocol = customAPI(api);
  const host = urlHost(baseURL);
  // The preset being added, else the one whose server the URL names or whose name the provider
  // has: for the key's hint.
  const preset = adding ?? presetForURL(baseURL) ?? customPreset(saved?.name);
  const requestURL = customRequestURL(api, baseURL);
  // On a phone, localhost is the phone: the server must be named as the Runners reach it.
  const local = !!adding?.local || isLoopbackHost(host);
  const urlNote = [
    requestURL ? t("Requests go to {url}.", { url: requestURL }) : t("Lorca adds {path} to it.", { path: protocol.path }),
    local ? t("Use the address of the computer running it, as your Runners reach it.") : undefined,
  ]
    .filter(Boolean)
    .join("\n");
  const picked = rows.filter((row) => row.selected);
  const defaultId = defaultModelId(rows, chosenDefault);
  const defaultRow = picked.find((row) => row.id === defaultId);
  // Left empty, the name is the preset's whose server the URL names, else the host.
  const fallbackName = defaultProviderName(baseURL);
  const providerName = name.trim() || fallbackName;
  const canSave = !working && !!providerName && !!baseURL.trim() && picked.length > 0;

  async function save() {
    if (!canSave) return;
    setWorking(true);
    setError(null);
    try {
      await engine.saveCustomProvider({ kind, name: providerName, api, baseURL, apiKey, models: selectedModelIds(rows, chosenDefault) });
      router.back();
    } catch (cause) {
      setError(messageOf(cause));
    } finally {
      setWorking(false);
    }
  }

  function confirmDelete() {
    if (!kind) return;
    alert(t("Delete {name}?", { name: saved?.name || providerName }), t("This removes the provider from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => {
          setWorking(true);
          setError(null);
          void engine
            .disconnectProvider(kind)
            .then(() => router.back())
            .catch((cause) => setError(messageOf(cause)))
            .finally(() => setWorking(false));
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: saved?.name || (adding ? t("Add {name}", { name: adding.name }) : t("Add Custom Provider")) }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        automaticallyAdjustKeyboardInsets
        contentContainerStyle={styles.content}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
      >
        <Section footer={t("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Encrypted and shared with your paired Devices.")}>
          <FieldRow
            label={t("Name")}
            value={name}
            onChangeText={setName}
            placeholder={fallbackName || "OpenRouter"}
            autoCapitalize="words"
            autoCorrect={false}
            editable={!working}
            returnKeyType="next"
          />
          <Row
            title={t("API")}
            menu={{
              title: t("API"),
              value: protocol.title,
              choices: CUSTOM_APIS.map((choice) => ({ title: choice.title, selected: choice.id === api, onPress: () => setAPI(choice.id) })),
            }}
          />
        </Section>

        <Section footer={urlNote}>
          <FieldRow
            label={t("Base URL")}
            value={baseURL}
            onChangeText={setBaseURL}
            placeholder={protocol.placeholder}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            editable={!working}
            returnKeyType="next"
          />
        </Section>

        <Section>
          <FieldRow
            label={t("API Key")}
            value={apiKey}
            onChangeText={setAPIKey}
            placeholder={preset ? preset.keyPlaceholder() : t("Optional for a server on your network")}
            secureTextEntry
            autoCapitalize="none"
            autoCorrect={false}
            editable={!working}
            returnKeyType="done"
          />
        </Section>

        <Section footer={modelListingNote(listing)}>
          <Row
            title={t("Models")}
            detail={picked.length ? t("{count} selected", { count: picked.length }) : t("None")}
            chevron
            onPress={() => router.push("/settings/custom-models")}
          />
          {defaultRow ? (
            <Row
              title={t("Default Model")}
              menu={{
                title: t("Default Model"),
                value: modelLabel(defaultRow),
                choices: picked.map((row) => ({ title: modelLabel(row), selected: row.id === defaultId, onPress: () => chooseDefaultModel(row.id) })),
              }}
            />
          ) : (
            <Row title={t("Default Model")} detail={t("None")} />
          )}
        </Section>

        <Section>
          <Row title={kind ? t("Save") : t("Add")} onPress={canSave ? () => void save() : undefined} accessory={working ? <ActivityIndicator /> : undefined} />
        </Section>

        {error ? <Text style={[styles.error, { color: p.red }]}>{error}</Text> : null}

        {kind ? (
          <Section>
            <Row title={t("Delete")} destructive onPress={!working ? confirmDelete : undefined} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  center: { flex: 1, alignItems: "center", justifyContent: "center" },
  error: { marginHorizontal: 32, marginTop: 10, fontSize: 13, lineHeight: 18 },
});
