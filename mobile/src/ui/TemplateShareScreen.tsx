// Shares a bot as a template, after the desktop apps' share sheet: the profile, which every
// template holds, then the skills, routines, plugins, and memories to pick from, each as the
// template will hold it. Share Link in the bar puts it on the relay behind a link and opens the
// share sheet on it; a bot shared before updates its link, which keeps its address. Save as File
// writes the same template for the share sheet. Details and Settings › Shared Links both slide
// it in, by the bot's id.

import { Stack } from "expo-router";
import * as Clipboard from "expo-clipboard";
import { File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import { useEffect, useState } from "react";
import { ActivityIndicator, Platform, ScrollView } from "react-native";
import { engine } from "../core/engine";
import { pathOf } from "../core/prefs";
import { useStore } from "../core/store";
import {
  allPicked,
  emptySelection,
  initialSelection,
  linkTitle,
  LONG_LIST,
  memoryLines,
  routineLine,
  sharedLinkFor,
  templateOrder,
  toggleAll,
  toggleItem,
  type TemplateContents,
  type TemplateKind,
  type TemplateSelection,
} from "../core/templates";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { daySeparator } from "./format";
import { Row, Section } from "./forms";
import { haptic } from "./haptics";
import { AndroidIcons } from "./navigation";
import { shareLink } from "./outputs";
import { TemplateItemRow, TemplateProfileRow } from "./templates";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function TemplateShareScreen({ botId }: { botId: string }) {
  useLanguage();
  const bot = useStore((s) => s.bots.find((each) => each.id === botId));
  const link = useStore((s) => sharedLinkFor(s.shared_links, botId));
  const routines = useStore((s) => s.routines);
  const [contents, setContents] = useState<TemplateContents>();
  const [failed, setFailed] = useState<string>();
  const [selection, setSelection] = useState<TemplateSelection>(emptySelection);
  const [busy, setBusy] = useState<"share" | "file" | "revoke">();
  /// The link holds what is picked now: said under the profile until the picks change.
  const [updated, setUpdated] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    engine
      .templateContents(botId)
      .then((loaded) => {
        setContents(loaded);
        setSelection(initialSelection(loaded, sharedLinkFor(useStore.getState().shared_links, botId)));
      })
      .catch((error) => setFailed(message(error)));
  }, [botId]);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  const name = bot?.name ?? link?.name ?? "";
  const order = contents ? templateOrder(contents) : undefined;

  function pick(next: TemplateSelection) {
    if (busy) return;
    setSelection(next);
    setUpdated(false);
  }

  /// Builds the template first, so the core's checks speak before anything leaves, then shares
  /// what was shown: the core refuses it if the contents changed since.
  async function share() {
    if (busy || !contents) return;
    setBusy("share");
    try {
      const shared = await engine.shareTemplate(botId, selection, link?.id);
      haptic.success();
      if (link) setUpdated(true);
      else shareLink(shared.url);
    } catch (error) {
      haptic.error();
      alert(link ? t("Couldn't update the link") : t("Couldn't share the bot"), message(error));
    } finally {
      setBusy(undefined);
    }
  }

  /// The same template as a file, for the share sheet to save or send.
  async function saveFile() {
    if (busy || !contents) return;
    setBusy("file");
    try {
      const file = new File(Paths.cache, `${name.replace(/[/\\:*?"<>|]/g, "-").trim() || "Bot"}.lorca-template`);
      await engine.saveTemplateFile(botId, selection, pathOf(file.uri));
      await Sharing.shareAsync(file.uri, { mimeType: "application/json", dialogTitle: name, UTI: "public.json" });
    } catch (error) {
      alert(t("Couldn't save the file"), message(error));
    } finally {
      setBusy(undefined);
    }
  }

  function confirmRevoke() {
    if (!link || busy) return;
    alert(t("Revoke the link to “{name}”?", { name }), t("Whoever opens it sees that it no longer works. Bots already added from it stay."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Revoke"),
        style: "destructive",
        onPress: async () => {
          setBusy("revoke");
          try {
            await engine.revokeLink(link.id);
            setUpdated(false);
          } catch (error) {
            alert(t("Couldn't revoke the link"), message(error));
          } finally {
            setBusy(undefined);
          }
        },
      },
    ]);
  }

  /// A kind's rows, Select All after a long list.
  function rows(kind: TemplateKind, items: { id: string; title: string; detail?: string; flags?: string[]; titleLines?: number }[]) {
    const ids = order![kind];
    return [
      ...items.map((item) => (
        <TemplateItemRow
          key={item.id}
          title={item.title}
          detail={item.detail}
          flags={item.flags}
          titleLines={item.titleLines}
          checked={selection[kind].includes(item.id)}
          onPress={() => pick(toggleItem(selection, kind, item.id, ids))}
        />
      )),
      items.length >= LONG_LIST ? <Row key="all" title={allPicked(selection, kind, ids) ? t("Deselect All") : t("Select All")} onPress={() => pick(toggleAll(selection, kind, ids))} /> : null,
    ];
  }

  const primary = link ? t("Update Link") : t("Share Link");
  const hint = busy === "share" ? (link ? t("Updating…") : t("Sharing…")) : updated ? t("The link now holds what you picked. Anyone who opens it gets this version.") : link ? t("Update the link with what you pick now. Its address stays the same.") : t("Others get a copy of what you pick. Keys, sign-ins, and chats stay.");

  return (
    <>
      <Stack.Screen options={{ title: t("Share Bot") }} />
      <Stack.Toolbar placement="right">
        {Platform.OS === "android" ? (
          <Stack.Toolbar.Button icon={AndroidIcons.link} accessibilityLabel={primary} disabled={!contents || !!busy} onPress={() => void share()} />
        ) : (
          <Stack.Toolbar.Button variant="done" disabled={!contents || !!busy} onPress={() => void share()}>
            {primary}
          </Stack.Toolbar.Button>
        )}
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        {link ? (
          <Section title={t("Link")} footer={t("Anyone with this link can add their own copy of {name}. Revoking it stops it working; copies already added stay.", { name })}>
            <Row title={linkTitle(link.url)} subtitle={t("Updated {date}", { date: daySeparator(new Date(link.updated_at * 1000)) })} icon="link" action={false} />
            <Row
              title={copied ? t("Copied") : t("Copy Link")}
              icon="doc.on.doc"
              onPress={async () => {
                await Clipboard.setStringAsync(link.url);
                haptic.success();
                setCopied(true);
              }}
            />
            <Row title={t("Share…")} icon="square.and.arrow.up" onPress={() => shareLink(link.url)} />
          </Section>
        ) : null}

        {!contents ? (
          <Section footer={failed}>
            <Row title={failed ? t("Couldn't read the bot") : t("Loading…")} accessory={failed ? undefined : <ActivityIndicator />} />
          </Section>
        ) : (
          <>
            <Section title={t("Profile")} footer={hint}>
              <TemplateProfileRow profile={contents.profile.content} flags={contents.profile.flags} />
            </Section>
            {contents.skills.length ? (
              <Section title={t("Skills")}>{rows("skill_ids", contents.skills.map((item) => ({ id: item.id, title: item.content.name, detail: item.content.description, flags: item.flags })))}</Section>
            ) : null}
            {contents.routines.length ? (
              <Section title={t("Routines")}>
                {rows(
                  "routine_ids",
                  contents.routines.map((item) => ({
                    id: item.id,
                    title: item.content.name,
                    detail: routineLine({ ...item.content, schedule_text: routines.find((routine) => routine.id === item.id)?.schedule_text }),
                    flags: item.flags,
                  })),
                )}
              </Section>
            ) : null}
            {contents.requirements.length ? (
              <Section title={t("Plugins")}>
                {rows("requirement_ids", contents.requirements.map((item) => ({ id: item.service_id, title: item.name })))}
              </Section>
            ) : null}
            {contents.memories.length ? (
              <Section title={t("Memories")}>
                {rows(
                  "memory_ids",
                  contents.memories.map((item) => ({ id: item.id, ...memoryLines(item.content), flags: item.flags, titleLines: 3 })),
                )}
              </Section>
            ) : null}
            <Section>
              <Row title={t("Save as File…")} icon="square.and.arrow.down" accessory={busy === "file" ? <ActivityIndicator /> : undefined} onPress={busy ? undefined : () => void saveFile()} />
            </Section>
          </>
        )}

        {link ? (
          <Section>
            <Row title={t("Revoke Link…")} icon="xmark.circle" destructive accessory={busy === "revoke" ? <ActivityIndicator /> : undefined} onPress={busy ? undefined : confirmRevoke} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}
