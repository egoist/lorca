// A skill of a bot's or a group's, slid in from its row in Details, after the desktop apps' skill
// sheet: who can use it, its name and when to use it, the instructions and an example, the
// reference and script files it carries, and how it changed. `new` as the id is the same form for
// a new skill; a draft a bot proposed or Save as Skill wrote opens here too, and is used once it is
// saved. When another Device saved first, the screen offers that version or to overwrite it, as
// the memory editor does on the desktop.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import { useEffect, useState } from "react";
import { Platform, ScrollView, StyleSheet } from "react-native";
import { chatTitle, engine } from "../../../src/core/engine";
import { isStaleSkill, type PlaybookRecord, type PlaybookScope } from "../../../src/core/model";
import { useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { daySeparator } from "../../../src/ui/format";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";
import { addSkillFile, fillSkillForm, historyWord, safeFileName, skillFormChanged, skillFormContent, skillNameProblem, skillScopeLine, useSkillForm } from "../../../src/ui/skills";
import { Font } from "../../../src/ui/theme";

export default function SkillScreen() {
  useLanguage();
  const { id, kind, scope: scopeId } = useLocalSearchParams<{ id: string; kind: PlaybookScope["kind"]; scope: string }>();
  const scope: PlaybookScope = { kind, id: scopeId };
  const router = useRouter();
  const form = useSkillForm();
  const [record, setRecord] = useState<PlaybookRecord | undefined>(() => (id === "new" ? undefined : engine.skills.get(id)));
  const [taken, setTaken] = useState<string>();
  const [busy, setBusy] = useState(false);
  const bot = useStore((s) => (scope.kind === "bot" ? s.bots.find((b) => b.id === scope.id) : undefined));
  const group = useStore((s) => (scope.kind === "project" ? s.chats.find((c) => c.id === scope.id) : undefined));
  const devices = useStore((s) => s.devices);

  // The form starts from the record, fetched when this phone has not got it yet.
  useEffect(() => {
    fillSkillForm(record?.content);
    if (id === "new" || record) return;
    engine
      .playbook(scope, id)
      .then((fetched) => {
        setRecord(fetched);
        fillSkillForm(fetched.content);
      })
      .catch((error) => alert(t("Couldn't open {name}", { name: id }), error instanceof Error ? error.message : String(error), [{ text: t("OK"), onPress: () => router.back() }]));
  }, [id]);

  if (id !== "new" && !record) return null;

  const content = skillFormContent(form);
  const nameProblem = skillNameProblem(form.name) ?? (taken && taken === content.name ? t("Another skill here has this name.") : undefined);
  const badFile = form.files.find((file) => !safeFileName(file.name) || form.files.some((other) => other !== file && other.folder === file.folder && other.name === file.name));
  const canSave = !busy && !!content.name && !nameProblem && !badFile && !!content.description && !!content.instructions.trim() && (!record || record.status === "draft" || skillFormChanged(form, record));
  const isDraft = record?.status === "draft";
  const scopeLine = skillScopeLine(scope, isDraft, { bot: bot?.name, group: group ? chatTitle(group) : undefined });

  async function save(over: PlaybookRecord | undefined) {
    if (busy) return;
    setBusy(true);
    try {
      await engine.savePlaybook(scope, skillFormContent(useSkillForm.getState()), over);
      router.back();
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (over && isStaleSkill(text)) resolveConflict(over);
      else if (text.includes("already exists")) setTaken(content.name);
      else if (text.includes("was removed")) alert(t("This skill was deleted on another Device."));
      else alert(t("Couldn't save the skill"), text);
    } finally {
      setBusy(false);
    }
  }

  /// The skill changed on another Device while it was open here: show that version, or save these
  /// edits over it.
  function resolveConflict(base: PlaybookRecord) {
    const load = async (then: (fresh: PlaybookRecord) => void) => {
      try {
        then(await engine.playbook(scope, base.id));
      } catch {
        alert(t("This skill was deleted on another Device."));
      }
    };
    alert(t("This skill changed on another Device"), t("Reload shows the latest version and discards your edits. Overwrite saves yours over it."), [
      {
        text: t("Reload"),
        onPress: () =>
          void load((fresh) => {
            setRecord(fresh);
            fillSkillForm(fresh.content);
          }),
      },
      { text: t("Overwrite with mine"), onPress: () => void load((fresh) => void save(fresh)) },
      { text: t("Cancel"), style: "cancel" },
    ]);
  }

  async function share() {
    if (!record?.content) return;
    try {
      const file = new File(Paths.cache, `${record.content.name}.json`);
      file.write(await engine.exportPlaybook(scope, record.id));
      await Sharing.shareAsync(file.uri, { mimeType: "application/json", dialogTitle: record.content.name, UTI: "public.json" });
    } catch (error) {
      alert(t("Couldn't export the skill"), error instanceof Error ? error.message : String(error));
    }
  }

  function confirmDelete() {
    if (!record) return;
    alert(t("Delete “{name}”?", { name: record.content?.name ?? "" }), isDraft ? t("The draft is deleted.") : t("Your bots stop using this skill. This can't be undone."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: async () => {
          try {
            await engine.removePlaybook(record);
            router.back();
          } catch (error) {
            alert(t("Couldn't delete the skill"), error instanceof Error ? error.message : String(error));
          }
        },
      },
    ]);
  }

  const steps = [...(record?.revisions ?? [])].sort((a, b) => b.revision - a.revision || b.created_at - a.created_at);
  const fileRows = (folder: "references" | "scripts") =>
    form.files
      .filter((file) => file.folder === folder)
      .map((file) => (
        <Row
          key={file.key}
          title={file.name}
          subtitle={file.text.split("\n")[0] || undefined}
          icon={folder === "scripts" ? "terminal" : "doc.text"}
          chevron
          onPress={() => router.push({ pathname: "/chat-info/skill-file", params: { key: String(file.key) } })}
        />
      ));
  const addFile = (folder: "references" | "scripts") => {
    const key = addSkillFile(folder);
    router.push({ pathname: "/chat-info/skill-file", params: { key: String(key) } });
  };

  return (
    <>
      <Stack.Screen options={{ title: record?.content?.name ?? t("New Skill") }} />
      <SaveToolbar label={t("Save")} disabled={!canSave} onSave={() => void save(record)} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section footer={nameProblem ?? scopeLine}>
          <FieldRow label={t("Name")} value={form.name} onChangeText={(name) => useSkillForm.setState({ name })} placeholder="weekly-report" autoCapitalize="none" autoCorrect={false} autoFocus={!record} style={styles.mono} />
          <FieldRow label={t("Description")} value={form.description} onChangeText={(description) => useSkillForm.setState({ description })} placeholder={t("When a bot should use it")} autoCapitalize="sentences" />
        </Section>
        <Section title={t("Instructions")}>
          <FieldRow value={form.instructions} onChangeText={(instructions) => useSkillForm.setState({ instructions })} placeholder={t("What to do, step by step")} multiline autoCapitalize="sentences" style={styles.text} />
        </Section>
        <Section title={t("Examples")}>
          <FieldRow value={form.examples} onChangeText={(examples) => useSkillForm.setState({ examples })} placeholder={t("A good result to aim for (optional)")} multiline autoCapitalize="sentences" style={styles.example} />
        </Section>
        <Section title={t("References")}>
          {fileRows("references")}
          <Row title={t("Add Reference")} icon="plus" onPress={() => addFile("references")} />
        </Section>
        <Section title={t("Scripts")}>
          {fileRows("scripts")}
          <Row title={t("Add Script")} icon="plus" onPress={() => addFile("scripts")} />
        </Section>
        {/* How the skill changed, newest first; a row shows its instructions then. */}
        {steps.length > 0 && (
          <Section title={t("History")}>
            {steps.map((step) => {
              const device = devices.find((d) => d.id === step.device_id)?.name;
              const when = daySeparator(new Date(step.created_at * 1000));
              return (
                <Row
                  key={step.id}
                  title={historyWord(step, steps)}
                  subtitle={device ? `${when} · ${device}` : when}
                  onPress={() => alert(historyWord(step, steps), step.content?.instructions ?? t("Deleted"))}
                  action={false}
                />
              );
            })}
          </Section>
        )}
        {record?.status === "saved" && (
          <Section>
            <Row title={t("Export")} icon="square.and.arrow.up" onPress={() => void share()} />
          </Section>
        )}
        {record && (
          <Section>
            <Row title={t("Delete")} icon="trash" destructive onPress={confirmDelete} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  mono: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: Font.body - 2 },
  text: { minHeight: 160 },
  example: { minHeight: 72 },
});
