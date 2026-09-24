import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ActivityIndicator, Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import {
  isProviderKind,
  providerDefaultBaseURL,
  providerKeyPlaceholder,
  providerLabel,
  providerSignInRequirement,
  providerUsesAPIKey,
} from "../../../src/core/model";
import { useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { usePalette } from "../../../src/ui/theme";

export default function ProviderSettingsScreen() {
  useLanguage();
  const params = useLocalSearchParams<{ kind: string }>();
  const rawKind = Array.isArray(params.kind) ? params.kind[0] : params.kind;
  const router = useRouter();
  const p = usePalette();
  const status = useStore((s) => s.providers.find((provider) => provider.kind === rawKind));
  const [apiKey, setAPIKey] = useState("");
  const [baseURL, setBaseURL] = useState(status?.base_url ?? "");
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!rawKind || !isProviderKind(rawKind)) {
    return (
      <>
        <Stack.Screen options={{ title: t("Provider") }} />
        <View style={styles.center}>
          <Text style={{ color: p.secondaryLabel }}>{t("Unknown provider")}</Text>
        </View>
      </>
    );
  }

  const kind = rawKind;
  const name = providerLabel(kind);
  const usesAPIKey = providerUsesAPIKey(kind);

  async function connect() {
    if (working || (usesAPIKey && !apiKey.trim())) return;
    setWorking(true);
    setError(null);
    try {
      await engine.connectProvider(kind, usesAPIKey ? { apiKey, baseURL } : undefined);
      router.back();
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      if (message !== "Sign-in cancelled") setError(message);
    } finally {
      setWorking(false);
    }
  }

  function confirmDisconnect() {
    Alert.alert(
      t("Disconnect {name}?", { name }),
      t("This removes the credential from every paired Device."),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: t("Disconnect"),
          style: "destructive",
          onPress: () => {
            setWorking(true);
            setError(null);
            void engine
              .disconnectProvider(kind)
              .then(() => router.back())
              .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)))
              .finally(() => setWorking(false));
          },
        },
      ],
    );
  }

  const connectTitle = usesAPIKey
    ? status?.is_connected
      ? t("Update Credential")
      : t("Connect")
    : t("Sign in with {name}…", { name });

  return (
    <>
      <Stack.Screen options={{ title: name }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={styles.content}
        keyboardDismissMode="on-drag"
      >
        <Section title={t("Status")}>
          <Row
            title={status?.is_connected ? t("Connected") : t("Not connected")}
            subtitle={status?.is_connected && status.detail ? status.detail : undefined}
            accessory={working ? <ActivityIndicator /> : undefined}
          />
        </Section>

        {usesAPIKey ? (
          <Section
            title={t("Credential")}
            footer={t("The key is checked with {name}, then shared with your paired Devices, encrypted with your account key.", { name })}
          >
            <FieldRow
              label={t("API Key")}
              value={apiKey}
              onChangeText={setAPIKey}
              placeholder={providerKeyPlaceholder(kind)}
              secureTextEntry
              autoCapitalize="none"
              autoCorrect={false}
              editable={!working}
              returnKeyType="next"
            />
            <FieldRow
              label={t("Base URL")}
              value={baseURL}
              onChangeText={setBaseURL}
              placeholder={providerDefaultBaseURL(kind)}
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              editable={!working}
              returnKeyType="done"
              onSubmitEditing={() => void connect()}
            />
            <Row title={connectTitle} onPress={!working && apiKey.trim() ? () => void connect() : undefined} />
          </Section>
        ) : (
          <Section
            title={t("Subscription")}
            footer={`${t("Your browser opens a {name} sign-in. The tokens are shared with your paired Devices, encrypted with your account key; the relay cannot read them.", { name })} ${providerSignInRequirement(kind)}`}
          >
            <Row title={connectTitle} onPress={!working ? () => void connect() : undefined} />
          </Section>
        )}

        {error ? <Text style={[styles.error, { color: p.red }]}>{error}</Text> : null}

        {status?.is_connected ? (
          <Section>
            <Row title={t("Disconnect")} destructive onPress={!working ? confirmDisconnect : undefined} />
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
