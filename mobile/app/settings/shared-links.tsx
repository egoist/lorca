// Settings › Shared Links, after the desktop apps' pane: the account's links, the newest change
// first, each with its bot and when it was last updated. A row slides in its bot's share screen,
// which copies, updates, and revokes the link; a bot deleted since keeps its link until it is
// revoked, and its row offers that.

import { Stack, useRouter } from "expo-router";
import * as Clipboard from "expo-clipboard";
import { ScrollView } from "react-native";
import { engine } from "../../src/core/engine";
import { useStore } from "../../src/core/store";
import { newestLinks, type SharedLink } from "../../src/core/templates";
import { t, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { BotAvatar } from "../../src/ui/Avatar";
import { daySeparator } from "../../src/ui/format";
import { Row, Section } from "../../src/ui/forms";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";

export default function SharedLinksScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const links = newestLinks(useStore((s) => s.shared_links));
  const bots = useStore((s) => s.bots);

  /// A deleted bot's link: copy it, or take it down.
  function showOrphan(link: SharedLink) {
    alert(link.name, link.url, [
      { text: t("Copy Link"), onPress: () => void Clipboard.setStringAsync(link.url) },
      {
        text: t("Revoke Link…"),
        style: "destructive",
        onPress: () =>
          alert(t("Revoke the link to “{name}”?", { name: link.name }), t("Whoever opens it sees that it no longer works. Bots already added from it stay."), [
            { text: t("Cancel"), style: "cancel" },
            {
              text: t("Revoke"),
              style: "destructive",
              onPress: () => engine.revokeLink(link.id).catch((error) => alert(t("Couldn't revoke the link"), error instanceof Error ? error.message : String(error))),
            },
          ]),
      },
      { text: t("Cancel"), style: "cancel" },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Shared Links") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          footer={
            links.length
              ? t("Anyone with a link can add their own copy of the bot. Revoking a link stops it working; copies already added stay.")
              : t("No shared links yet. Share a bot from its Details.")
          }
        >
          {links.map((link) => {
            const bot = bots.find((each) => each.id === link.bot_id);
            return (
              <Row
                key={link.id}
                title={bot?.name ?? link.name}
                subtitle={t("Updated {date}", { date: daySeparator(new Date(link.updated_at * 1000)) })}
                leading={bot ? <BotAvatar bot={bot} size={32} /> : <Symbol name="link" size={20} color={p.secondaryLabel} />}
                chevron={!!bot}
                action={false}
                onPress={() => (bot ? router.push(`/settings/shared-link/${bot.id}`) : showOrphan(link))}
              />
            );
          })}
        </Section>
      </ScrollView>
    </>
  );
}
