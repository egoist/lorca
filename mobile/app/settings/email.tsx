// Settings › Email, after the desktop apps' Email pane: the account's one address, which every
// bot shares. Without one, a row gets it. With one, a tap on the address copies it, Change Address
// slides in the screen that picks a new name, and Give Up Address lets it go after a confirmation:
// mail to it bounces from then on. The footer says which bot gets the mail.

import * as Clipboard from "expo-clipboard";
import { Stack, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text } from "react-native";
import { engine } from "../../src/core/engine";
import { mailRoutingNote } from "../../src/core/mail";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { Row, Section } from "../../src/ui/forms";
import { haptic } from "../../src/ui/haptics";
import { Font, usePalette } from "../../src/ui/theme";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export default function EmailScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const mail = useStore((s) => s.mail);
  const bots = useStore((s) => s.bots);
  const [copied, setCopied] = useState(false);
  const [working, setWorking] = useState(false);

  // Where the address stands now: another Device may have changed it while this one was away.
  useEffect(() => {
    engine.refreshMail().catch((error) => console.warn("reading the email address", message(error)));
  }, []);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  const address = mail?.address;

  async function copy() {
    if (!address) return;
    await Clipboard.setStringAsync(address.email);
    haptic.success();
    setCopied(true);
  }

  function confirmGiveUp() {
    if (!address || working) return;
    alert(t("Give up {email}?", { email: address.email }), t("Mail to it will bounce, and nobody else can take the name."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Give Up"),
        style: "destructive",
        onPress: async () => {
          setWorking(true);
          try {
            await engine.releaseMail();
          } catch (error) {
            alert(t("Couldn't give up the address"), message(error));
          } finally {
            setWorking(false);
          }
        },
      },
    ]);
  }

  const suspended = address?.state === "suspended";
  const footer = suspended ? t("Mail to this address bounces for now, because mail from it kept bouncing.") : mail ? mailRoutingNote(mail, bots) : undefined;

  return (
    <>
      <Stack.Screen options={{ title: t("Email") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        {!mail ? (
          <Section>
            <Row title={t("Loading…")} accessory={<ActivityIndicator />} />
          </Section>
        ) : !mail.available ? null : address ? (
          <>
            <Section footer={footer}>
              <Row
                title={address.email}
                action={false}
                onPress={() => void copy()}
                detail={copied ? t("Copied") : undefined}
                // The one state the user needs to know about, in red as the Mac's row has it.
                accessory={!copied && suspended ? <Text style={[styles.status, { color: p.red }]}>{t("Suspended")}</Text> : undefined}
              />
              <Row title={t("Change Address")} onPress={working ? undefined : () => router.push("/settings/email-address")} />
            </Section>
            <Section>
              <Row title={t("Give Up Address")} destructive onPress={working ? undefined : confirmGiveUp} accessory={working ? <ActivityIndicator /> : undefined} />
            </Section>
          </>
        ) : (
          <Section footer={t("One address all your bots share: they use it to sign up for services, write to people for you, and schedule time. Lorca keeps the mail readable only by your Runners.")}>
            <Row title={t("Get an Address")} onPress={() => router.push("/settings/email-address")} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  status: { fontSize: Font.body },
});
