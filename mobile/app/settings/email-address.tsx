// Gets the account an email address, or changes the one it has, slid in from Settings › Email:
// a random name, or one of the user's own on the relay's mail domain if no one has it. The core
// checks the name again and says why it can't have it; changing gives the old name up.

import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView, StyleSheet, Text } from "react-native";
import { engine } from "../../src/core/engine";
import { mailNameIsValid, mailProblemText, normalizeMailName, type MailProblem } from "../../src/core/mail";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { CheckRow, FieldRow, Section } from "../../src/ui/forms";
import { haptic } from "../../src/ui/haptics";
import { SaveToolbar } from "../../src/ui/navigation";
import { usePalette } from "../../src/ui/theme";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export default function EmailAddressScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const mail = useStore((s) => s.mail);
  // The address as it stood when the screen opened, so the title and the button stay put while
  // the change lands in the store.
  const [current] = useState(mail?.address ?? null);
  const [own, setOwn] = useState(false);
  const [name, setName] = useState("");
  const [problem, setProblem] = useState<MailProblem>();
  const [error, setError] = useState<string>();
  const [working, setWorking] = useState(false);

  const typed = normalizeMailName(name);
  const domain = mail?.domain ?? "";
  // A character the address can't hold is said at once; a name too short only stops the button.
  const badCharacters = !!typed && (/[^a-z0-9.-]/.test(typed) || /^[.-]/.test(typed) || typed.includes(".."));
  const canSave = !working && (!own || (mailNameIsValid(typed) && typed !== current?.name));
  const note = problem ? mailProblemText(problem) : own && badCharacters ? mailProblemText("invalid") : error;

  async function save() {
    if (!canSave) return;
    setWorking(true);
    setProblem(undefined);
    setError(undefined);
    try {
      const refused = await engine.applyMail(own ? typed : undefined);
      if (refused) {
        haptic.error();
        setProblem(refused);
      } else {
        haptic.success();
        router.back();
      }
    } catch (cause) {
      haptic.error();
      setError(message(cause));
    } finally {
      setWorking(false);
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: current ? t("Change Address") : t("Get an Address") }} />
      <SaveToolbar label={current ? t("Change") : t("Get Address")} disabled={!canSave} onSave={() => void save()} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        automaticallyAdjustKeyboardInsets
        contentContainerStyle={{ paddingBottom: 40 }}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
      >
        <Section footer={current ? t("Mail to {email} bounces once you change it.", { email: current.email }) : undefined}>
          <CheckRow title={t("Random Name")} subtitle={t("Like {email}", { email: `x4p9t2wq@${domain}` })} checked={!own} onPress={() => setOwn(false)} disabled={working} />
          <CheckRow title={t("Your Own Name")} checked={own} onPress={() => setOwn(true)} disabled={working} />
        </Section>

        {own ? (
          <Section footer={note ? undefined : t("3 to 32 letters, digits, dots, or hyphens.")}>
            <FieldRow
              label={t("Name")}
              value={name}
              onChangeText={(text) => {
                setName(text.toLowerCase());
                setProblem(undefined);
                setError(undefined);
              }}
              placeholder={t("name")}
              suffix={`@${domain}`}
              autoFocus
              autoCapitalize="none"
              autoCorrect={false}
              autoComplete="off"
              keyboardType="ascii-capable"
              editable={!working}
              returnKeyType="done"
              onSubmitEditing={() => void save()}
              style={{ textAlign: "right" }}
            />
          </Section>
        ) : null}

        {note ? <Text style={[styles.note, { color: p.red }]}>{note}</Text> : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  // Where a section's footer sits, for a line the user has to act on.
  note: { marginHorizontal: 32, marginTop: 7, fontSize: 13, lineHeight: 18 },
});
