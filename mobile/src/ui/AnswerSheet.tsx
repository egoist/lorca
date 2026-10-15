// The answer to a command waiting for input in its terminal: a bottom sheet that holds only the
// field, with the command's question as its placeholder, and Send. `@expo/ui`'s sheet on both
// platforms, SwiftUI's on iOS and Material's on Android. The field takes the text as typed, with
// no capitalization or correction, which would change a password, and hides it unless the
// question is a yes or no.

import { Host as ComposeHost, Icon as ComposeIcon, IconButton, ModalBottomSheet, OutlinedTextField, Text as ComposeText } from "@expo/ui/jetpack-compose";
import { fillMaxWidth, padding as composePadding } from "@expo/ui/jetpack-compose/modifiers";
import { BottomSheet, Button as SwiftButton, Group, HStack, Host as SwiftHost, Image as SwiftImage, SecureField, Text as SwiftText, TextField, VStack } from "@expo/ui/swift-ui";
import { accessibilityLabel, autocorrectionDisabled, background, buttonBorderShape, buttonStyle, disabled, font, foregroundStyle, onSubmit, padding, presentationDragIndicator, shapes, submitLabel, textInputAutocapitalization } from "@expo/ui/swift-ui/modifiers";
import { useState } from "react";
import { Platform, StyleSheet } from "react-native";
import type { CommandRun } from "../core/model";
import { t, useLanguage } from "../i18n";
import { AndroidIcons } from "./navigation";
import { usePalette } from "./theme";

function asksYesOrNo(run: CommandRun): boolean {
  return /\[y\/n\]|\(y\/n\)|\(yes\/no/i.test(run.prompt ?? "");
}

/// Up while mounted. `onSend` rejects with why the answer did not go; the sheet shows it under the
/// field and stays.
export function AnswerSheet({ run, onSend, onDismiss }: { run: CommandRun; onSend: (text: string) => Promise<void>; onDismiss: () => void }) {
  useLanguage();
  const p = usePalette();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const secret = !asksYesOrNo(run);
  const placeholder = run.prompt?.trim() || t("Type your answer");
  const send = async () => {
    if (sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(text);
      onDismiss();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  if (Platform.OS === "android") {
    return (
      <ComposeHost style={StyleSheet.absoluteFill} pointerEvents="box-none">
        <ModalBottomSheet onDismissRequest={onDismiss} containerColor={p.cell} contentColor={p.label}>
          <OutlinedTextField
            autoFocus
            singleLine
            isError={error != null}
            onValueChange={setText}
            visualTransformation={secret ? "password" : "none"}
            keyboardOptions={{ keyboardType: secret ? "password" : "text", capitalization: "none", autoCorrectEnabled: false, imeAction: "send" }}
            keyboardActions={{ onSend: () => void send() }}
            modifiers={[fillMaxWidth(), composePadding(16, 0, 16, 24)]}
          >
            <OutlinedTextField.Placeholder>
              <ComposeText>{placeholder}</ComposeText>
            </OutlinedTextField.Placeholder>
            <OutlinedTextField.TrailingIcon>
              <IconButton enabled={!sending} onClick={() => void send()}>
                <ComposeIcon source={AndroidIcons.send} size={24} />
              </IconButton>
            </OutlinedTextField.TrailingIcon>
            {error ? (
              <OutlinedTextField.SupportingText>
                <ComposeText>{error}</ComposeText>
              </OutlinedTextField.SupportingText>
            ) : null}
          </OutlinedTextField>
        </ModalBottomSheet>
      </ComposeHost>
    );
  }

  const field = [textInputAutocapitalization("never"), autocorrectionDisabled(), submitLabel("send"), onSubmit(() => void send())];
  return (
    <SwiftHost style={styles.host}>
      <BottomSheet isPresented onIsPresentedChange={(presented) => !presented && onDismiss()} fitToContents>
        <Group modifiers={[presentationDragIndicator("visible")]}>
          <VStack alignment="leading" spacing={8} modifiers={[padding({ horizontal: 16, top: 26, bottom: 16 })]}>
            <HStack spacing={8} modifiers={[padding({ leading: 16, trailing: 5, top: 5, bottom: 5 }), background(p.fill, shapes.capsule())]}>
              {secret ? (
                <SecureField autoFocus placeholder={placeholder} onTextChange={setText} modifiers={field} />
              ) : (
                <TextField autoFocus placeholder={placeholder} onTextChange={setText} modifiers={field} />
              )}
              <SwiftButton onPress={() => void send()} modifiers={[buttonStyle("glassProminent"), buttonBorderShape("circle"), disabled(sending), accessibilityLabel(t("Send"))]}>
                <SwiftImage systemName="arrow.up" />
              </SwiftButton>
            </HStack>
            {error ? <SwiftText modifiers={[font({ textStyle: "footnote" }), foregroundStyle("red"), padding({ horizontal: 16 })]}>{error}</SwiftText> : null}
          </VStack>
        </Group>
      </BottomSheet>
    </SwiftHost>
  );
}

const styles = StyleSheet.create({
  host: { position: "absolute", width: 0, height: 0 },
});
