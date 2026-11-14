// One of a channel's intake rules, slid in from the channel's screen: its name and the kind of work
// it covers, in the owner's words, which Save writes back. Without a `rule` the same form adds one,
// and the CLI gives it its id.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView } from "react-native";
import { engine } from "../../../src/core/engine";
import { useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { intakeWithoutRule, intakeWithRule } from "../../../src/ui/channels";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";

export default function ChannelRuleScreen() {
  useLanguage();
  const { channel: channelId, rule: ruleId } = useLocalSearchParams<{ channel: string; rule?: string }>();
  const router = useRouter();
  const intake = useStore((s) => s.devices.find((d) => d.channels?.some((c) => c.id === channelId))?.channels?.find((c) => c.id === channelId)?.intake);
  const rule = ruleId ? intake?.rules?.find((each) => each.id === ruleId) : undefined;
  const [name, setName] = useState(rule?.name ?? "");
  const [instruction, setInstruction] = useState(rule?.instruction ?? "");
  const [saving, setSaving] = useState(false);
  if (!intake) return null;
  const current = intake;

  const words = { name: name.trim(), instruction: instruction.trim() };
  const canSave = !saving && !!words.name && !!words.instruction;

  async function save() {
    if (!canSave) return;
    if (rule && words.name === rule.name && words.instruction === rule.instruction) return router.back();
    setSaving(true);
    try {
      await engine.setChannelIntake(channelId, intakeWithRule(current, { id: ruleId ?? "", enabled: rule?.enabled ?? true, ...words }));
      router.back();
    } catch (error) {
      setSaving(false);
      alert(t("Couldn’t save the rule"), error instanceof Error ? error.message : String(error));
    }
  }

  function confirmDelete() {
    alert(t("Delete “{name}”?", { name: rule?.name ?? "" }), undefined, [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => {
          engine.setChannelIntake(channelId, intakeWithoutRule(current, ruleId ?? "")).catch((error) => alert(t("Couldn’t save the rule"), error instanceof Error ? error.message : String(error)));
          router.back();
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: rule ? t("Rule") : t("New Rule") }} />
      <SaveToolbar label={rule ? t("Save") : t("Add")} disabled={!canSave} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section>
          <FieldRow label={t("Name")} value={name} onChangeText={setName} placeholder={t("What it covers")} autoCapitalize="sentences" maxLength={60} />
          <FieldRow label={t("Instruction")} value={instruction} onChangeText={setInstruction} placeholder={t("What to take on, and how far to go")} autoCapitalize="sentences" multiline maxLength={1000} />
        </Section>
        {rule && (
          <Section>
            <Row title={t("Delete Rule…")} destructive onPress={confirmDelete} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}
