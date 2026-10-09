// The marketplace's workflows, from the chat list's New menu, as the Mac's Choose a Workflow: each
// with its symbol and outcome, sliding in its setup.

import { Stack, useRouter } from "expo-router";
import { useCallback, useEffect, useState } from "react";
import { Platform, ScrollView, StyleSheet } from "react-native";
import { loadWorkflowPacks } from "../../src/core/workflows";
import { t, useLanguage } from "../../src/i18n";
import { Row, Section } from "../../src/ui/forms";
import { CloseToolbar } from "../../src/ui/navigation";
import { PackTile } from "../../src/ui/PackTile";
import type { WorkflowPack } from "../../src/ui/workflows";

export default function WorkflowsScreen() {
  useLanguage();
  const router = useRouter();
  const [packs, setPacks] = useState<WorkflowPack[]>();
  const [failed, setFailed] = useState(false);

  const load = useCallback(() => {
    setFailed(false);
    loadWorkflowPacks().then(setPacks, () => setFailed(true));
  }, []);
  useEffect(load, [load]);

  return (
    <>
      <Stack.Screen options={{ title: t("Choose a Workflow") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        {packs?.length ? (
          <Section>
            {packs.map((pack) => (
              <Row
                key={pack.id}
                title={pack.name}
                subtitle={pack.outcome}
                subtitleLines={2}
                leading={<PackTile symbol={pack.symbol_name} size={40} />}
                chevron
                onPress={() => router.push({ pathname: "/workflows/[id]", params: { id: pack.id } })}
              />
            ))}
          </Section>
        ) : failed ? (
          <Section footer={t("The marketplace isn't available right now.")}>
            <Row title={t("Try Again")} action onPress={load} />
          </Section>
        ) : (
          <Section>
            <Row title={packs ? t("Nothing here yet.") : t("Loading…")} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
});
