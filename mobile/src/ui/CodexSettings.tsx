import { useEffect, useState } from "react";
import { Alert } from "react-native";
import { engine } from "../core/engine";
import type { CodexCatalog, CodexSelection } from "../core/model";
import { t } from "../i18n";
import { Row } from "./forms";

export function CodexSettings({ runnerId, botId, selection, onChange }: {
  runnerId: string;
  botId?: string;
  selection: CodexSelection;
  onChange: (selection: CodexSelection) => void | Promise<void>;
}) {
  const [catalog, setCatalog] = useState<CodexCatalog>();
  const [error, setError] = useState<string>();
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    setCatalog(undefined);
    setError(undefined);
    void engine.codexModels(runnerId, botId).then((value) => { if (active) setCatalog(value); })
      .catch((error) => { if (active) setError(String(error)); });
    return () => { active = false; };
  }, [runnerId, botId, revision]);
  const model = catalog?.models.find((m) => m.id === (selection.model ?? catalog.default_model));
  const defaultModel = catalog?.default_model ? t("Default ({model})", { model: catalog.default_model }) : t("Default");
  const defaultThinking = model?.levels.includes(catalog?.default_thinking ?? "") ? catalog?.default_thinking : model?.default_thinking;
  const defaultThinkingLabel = defaultThinking ? t("Default ({model})", { model: defaultThinking }) : t("Default");
  const models = catalog?.models ?? [];
  const levels = model?.levels ?? [];
  const apply = async (value: CodexSelection) => {
    try { await onChange(value); }
    catch (error) { Alert.alert(t("Could not update the bot"), String(error)); }
  };
  const speeds = [
    { id: "default" as const, label: catalog?.default_service_tier === "priority" && (!model || model.fast_tier) ? t("Default ({model})", { model: "Fast" }) : t("Default") },
    { id: "standard" as const, label: t("Standard") },
    ...(model?.fast_tier || selection.options.speed === "fast" ? [{ id: "fast" as const, label: t("Fast") }] : []),
  ];
  return <>
    {catalog && <>
      <Row title={t("Model")} menu={{ title: t("Model"), value: selection.model ? model?.label ?? selection.model : defaultModel,
        choices: [{ id: undefined, label: defaultModel }, ...models].map((m) => ({ title: m.label, selected: selection.model === m.id,
          onPress: () => void apply({ ...selection, model: m.id, thinking: undefined,
            options: { ...selection.options, speed: selection.options.speed === "fast" && !models.find((item) => item.id === (m.id ?? catalog.default_model))?.fast_tier ? "default" : selection.options.speed } }) })) }} />
      <Row title={t("Thinking")} menu={{ title: t("Thinking"), value: selection.thinking ?? defaultThinkingLabel,
        choices: [undefined, ...levels].map((level) => ({ title: level ?? defaultThinkingLabel, selected: selection.thinking === level,
          onPress: () => void apply({ ...selection, thinking: level }) })) }} />
    </>}
    <Row title={t("Speed")} menu={{ title: t("Speed"), value: speeds.find((s) => s.id === selection.options.speed)?.label ?? t("Default"),
      choices: speeds.map((speed) => ({ title: speed.label, selected: speed.id === selection.options.speed,
        onPress: () => void apply({ ...selection, options: { ...selection.options, speed: speed.id } }) })) }} />
    <Row title={t("Approvals")} menu={{ title: t("Approvals"), value: selection.options.approvals === "auto_review" ? t("Approve for me") : t("Ask me"),
      choices: ([{ id: "auto_review", label: t("Approve for me") }, { id: "user", label: t("Ask me") }] as const).map((mode) => ({
        title: mode.label, selected: selection.options.approvals === mode.id,
        onPress: () => void apply({ ...selection, options: { ...selection.options, approvals: mode.id } }) })) }} />
    <Row title={t("Reload models")} subtitle={error ?? (catalog ? t("Fast uses more quota. Approve for me lets Codex review native approval requests. Changes apply to the next turn.") : t("Loading models…"))}
      onPress={() => setRevision((value) => value + 1)} />
  </>;
}
