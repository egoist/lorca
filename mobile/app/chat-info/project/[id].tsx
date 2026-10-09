// One entry of a group's project context, slid in from its row in Group Info, after the desktop
// apps' entry sheet: the title, the text, and the link it came from, or the file it holds, which
// opens in Quick Look. `new` as the id, with `kind`, is the same form for a new entry. Saving
// writes a new version every bot in the group reads from its next turn: Add, Save, Accept for a
// bot's suggestion, or Keep This Version for an entry two Devices changed at once. Remove sits
// apart, below. When another Device saved first, the screen offers that version or to overwrite
// it, as the memory editor does on the desktop.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Platform, PlatformColor, ScrollView, StyleSheet } from "react-native";
import { engine } from "../../../src/core/engine";
import { canCheckLink, fileSize, isSuggestion, linkChecked, otherVersions, STALE_PROJECT_ENTRY, type ProjectEntry, type ProjectKind } from "../../../src/core/model";
import { useProject } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { daySeparator } from "../../../src/ui/format";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";
import { openFile } from "../../../src/ui/outputs";
import { projectKindTitle, projectNewTitle, projectProvenance } from "../../../src/ui/project";
import { Symbol } from "../../../src/ui/Symbol";
import { accentColor, usePalette } from "../../../src/ui/theme";

interface Form {
  title: string;
  text: string;
  link: string;
}

const formOf = (entry: ProjectEntry | undefined): Form => ({ title: entry?.title ?? "", text: entry?.text ?? "", link: entry?.source.url ?? "" });

/// Why the entry can't be saved as it stands, in words for the form.
function whatStopsSaving(form: Form): string | undefined {
  const link = form.link.trim();
  if (new TextEncoder().encode(form.text).length > 32_000) return t("This is too long for one entry. Split it into a few.");
  if (new TextEncoder().encode(form.title).length > 240) return t("The title is too long.");
  if (link && !/^https:\/\/[^/\s]+/.test(link)) return t("Links start with https://.");
  return undefined;
}

export default function ProjectEntryScreen() {
  useLanguage();
  const { id, chat: chatId, kind: kindParam } = useLocalSearchParams<{ id: string; chat: string; kind?: ProjectKind }>();
  const router = useRouter();
  const p = usePalette();
  const project = useProject(chatId);
  // The version the form's edits start from, and the other versions saving it replaces.
  const [entry, setEntry] = useState<ProjectEntry | undefined>(() => (id === "new" ? undefined : project?.entries.find((each) => each.id === id)));
  const [others, setOthers] = useState<string[]>(() => otherVersions(project, id));
  const kind: ProjectKind = entry?.kind ?? kindParam ?? "brief";
  const [form, setForm] = useState<Form>(() => formOf(entry));
  const [saved, setSaved] = useState<Form>(() => formOf(entry));
  const [busy, setBusy] = useState(false);
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);

  if (id !== "new" && !entry) return null;

  const link = form.link.trim();
  const edited = form.title !== saved.title || form.text !== saved.text || link !== saved.link;
  const problem = whatStopsSaving(form);
  const changes = !entry || edited || isSuggestion(entry) || others.length > 0;
  const canSave = !busy && !!form.title.trim() && !(kind === "document" && !link) && !problem && changes;
  const confirm = !entry ? t("Add") : others.length > 0 ? t("Keep This Version") : isSuggestion(entry) ? t("Accept") : t("Save");

  function show(newer: ProjectEntry | undefined) {
    setEntry(newer);
    setForm(formOf(newer));
    setSaved(formOf(newer));
  }

  async function save(base: ProjectEntry | undefined) {
    if (busy) return;
    setBusy(true);
    try {
      await engine.saveProjectEntry(chatId, { kind, title: form.title.trim(), text: form.text, link: link || undefined, replacing: base, alsoReplacing: base ? others : [] });
      router.back();
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (base && text === STALE_PROJECT_ENTRY) await resolveConflict(base);
      else alert(t("Couldn't save this entry"), text);
    } finally {
      setBusy(false);
    }
  }

  /// Another Device changed or removed the entry while it was open here: show its version, or
  /// save these edits over it; a removed one can be added back.
  async function resolveConflict(base: ProjectEntry) {
    const newer = await engine.currentProjectEntry(chatId, base.id).catch(() => undefined);
    setOthers([]);
    if (newer) {
      alert(t("This entry changed on another Device"), t("Reload shows the other version and discards your edits. Overwrite saves yours over it."), [
        { text: t("Reload"), onPress: () => show(newer) },
        { text: t("Overwrite with mine"), onPress: () => void save(newer) },
        { text: t("Cancel"), style: "cancel" },
      ]);
    } else {
      alert(t("This entry was removed on another Device"), t("Add it back with your edits, or cancel to leave it removed."), [
        { text: t("Add It Back"), onPress: () => void save(undefined) },
        { text: t("Cancel"), style: "cancel" },
      ]);
    }
  }

  async function checkLink() {
    if (!entry || busy) return;
    setBusy(true);
    try {
      show(await engine.checkProjectLink(chatId, entry.id));
    } catch (error) {
      alert(t("Couldn't check this link"), error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }

  function confirmRemove() {
    if (!entry) return;
    alert(t("Remove “{name}”?", { name: entry.title }), t("Bots in this group stop seeing it."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Remove"),
        style: "destructive",
        onPress: async () => {
          try {
            await engine.removeProjectEntry(chatId, entry);
            router.back();
          } catch (error) {
            const text = error instanceof Error ? error.message : String(error);
            if (text === STALE_PROJECT_ENTRY) alert(t("This entry changed on another Device"), t("Close it and open it again to see the other version."));
            else alert(t("Couldn't remove this entry"), text);
          }
        },
      },
    ]);
  }

  // The link as the core last read it, for an entry that has one and while it is unchanged.
  const shownLink = entry?.source.url && entry.source.url === link ? entry : undefined;
  const checked = shownLink ? linkChecked(shownLink) : undefined;
  const linkStatus = !shownLink
    ? undefined
    : shownLink.freshness === "unavailable"
      ? t("Couldn't open this link. Bots use the copy from before.")
      : checked
        ? t("Checked {date}", { date: daySeparator(new Date(checked * 1000)) })
        : t("Not checked yet");

  const titleField = <FieldRow label={t("Title")} value={form.title} onChangeText={(title) => setForm((f) => ({ ...f, title }))} autoFocus={!entry} autoCapitalize="sentences" />;
  const linkField = (
    <FieldRow
      label={t("Link")}
      value={form.link}
      onChangeText={(value) => setForm((f) => ({ ...f, link: value }))}
      placeholder={kind === "document" ? "https://…" : t("Optional")}
      keyboardType="url"
      autoCapitalize="none"
      autoCorrect={false}
    />
  );
  const checkRow = shownLink && canCheckLink(shownLink) && !edited && !busy ? <Row title={t("Check Now")} icon="arrow.clockwise" onPress={() => void checkLink()} /> : null;
  const textField = <FieldRow value={form.text} onChangeText={(text) => setForm((f) => ({ ...f, text }))} multiline autoCapitalize="sentences" style={kind === "document" || kind === "asset" ? styles.notes : styles.text} />;

  return (
    <>
      <Stack.Screen options={{ title: entry ? projectKindTitle(kind) : projectNewTitle(kind) }} />
      <SaveToolbar label={confirm} disabled={!canSave} onSave={() => void save(entry)} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        {others.length > 0 && (
          <Section>
            <Row
              title={t("Two versions")}
              subtitle={t("Two Devices changed this at the same time. Keep This Version saves this one and replaces the other.")}
              subtitleLines={4}
              leading={<Symbol name="exclamationmark.circle.fill" size={20} color={orange} />}
            />
          </Section>
        )}

        {kind === "document" ? (
          <>
            <Section footer={problem ?? (entry ? projectProvenance(entry) : t("Every bot in this group can read it."))}>
              {titleField}
            </Section>
            <Section footer={linkStatus}>
              {linkField}
              {checkRow}
            </Section>
            <Section>{textField}</Section>
          </>
        ) : kind === "asset" ? (
          <Section footer={problem ?? (entry ? projectProvenance(entry) : undefined)}>
            {titleField}
            {entry?.asset ? (
              <Row title={entry.asset.name} subtitle={fileSize(entry.asset.size)} icon="doc" chevron onPress={() => void openFile(entry.asset!)} />
            ) : null}
            {textField}
          </Section>
        ) : (
          <>
            <Section footer={problem ?? (entry ? projectProvenance(entry) : t("Every bot in this group can read it."))}>
              {titleField}
              {textField}
            </Section>
            <Section footer={linkStatus}>
              {linkField}
              {checkRow}
            </Section>
          </>
        )}

        {entry && (
          <Section>
            <Row title={t("Remove")} icon="trash" destructive onPress={confirmRemove} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  text: { minHeight: 160 },
  notes: { minHeight: 96 },
});
