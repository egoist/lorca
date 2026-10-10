// The answer to a secret request, or a new value for a saved secret: a bottom sheet that holds only
// a field for each value, labelled by its placeholder, and Save beside the last. `@expo/ui`'s sheet
// on both platforms, SwiftUI's on iOS and Material's on Android, as the command's `AnswerSheet`.
// The fields hide what is typed and take it as typed, with no capitalization or correction.

import { Host as ComposeHost, Icon as ComposeIcon, IconButton, ModalBottomSheet, OutlinedTextField, Text as ComposeText } from "@expo/ui/jetpack-compose";
import { fillMaxWidth, padding as composePadding } from "@expo/ui/jetpack-compose/modifiers";
import { BottomSheet, Button as SwiftButton, Group, HStack, Host as SwiftHost, Image as SwiftImage, SecureField, Text as SwiftText, VStack } from "@expo/ui/swift-ui";
import { accessibilityLabel, autocorrectionDisabled, background, buttonBorderShape, buttonStyle, disabled, font, foregroundStyle, frame, onSubmit, padding, presentationDragIndicator, shapes, submitLabel, textInputAutocapitalization } from "@expo/ui/swift-ui/modifiers";
import { useState } from "react";
import { Platform, StyleSheet } from "react-native";
import { t, useLanguage } from "../i18n";
import { AndroidIcons } from "./navigation";
import { usePalette } from "./theme";

/// Up while mounted. `onSave` gets the values by field name and rejects with why they did not go;
/// the sheet shows it under the fields and stays.
export function SecretSheet({ fields, onSave, onDismiss }: { fields: { name: string; label: string }[]; onSave: (values: Record<string, string>) => Promise<void>; onDismiss: () => void }) {
  useLanguage();
  const p = usePalette();
  const [values, setValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const filled = fields.every((field) => (values[field.name] ?? "").trim() !== "");
  const set = (name: string) => (text: string) => {
    setError(null);
    setValues((current) => ({ ...current, [name]: text }));
  };
  const save = async () => {
    if (saving || !filled) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(values);
      onDismiss();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  if (Platform.OS === "android") {
    return (
      <ComposeHost style={StyleSheet.absoluteFill} pointerEvents="box-none">
        <ModalBottomSheet onDismissRequest={onDismiss} containerColor={p.cell} contentColor={p.label}>
          {fields.map((field, index) => {
            const last = index === fields.length - 1;
            return (
              <OutlinedTextField
                key={field.name}
                autoFocus={index === 0}
                singleLine
                isError={last && error != null}
                onValueChange={set(field.name)}
                visualTransformation="password"
                keyboardOptions={{ keyboardType: "password", capitalization: "none", autoCorrectEnabled: false, imeAction: last ? "done" : "next" }}
                keyboardActions={last ? { onDone: () => void save() } : undefined}
                modifiers={[fillMaxWidth(), composePadding(16, 0, 16, last ? 24 : 8)]}
              >
                <OutlinedTextField.Placeholder>
                  <ComposeText>{field.label}</ComposeText>
                </OutlinedTextField.Placeholder>
                {last ? (
                  <OutlinedTextField.TrailingIcon>
                    <IconButton enabled={!saving && filled} onClick={() => void save()}>
                      <ComposeIcon source={AndroidIcons.send} size={24} />
                    </IconButton>
                  </OutlinedTextField.TrailingIcon>
                ) : null}
                {last && error ? (
                  <OutlinedTextField.SupportingText>
                    <ComposeText>{error}</ComposeText>
                  </OutlinedTextField.SupportingText>
                ) : null}
              </OutlinedTextField>
            );
          })}
        </ModalBottomSheet>
      </ComposeHost>
    );
  }

  const field = [textInputAutocapitalization("never"), autocorrectionDisabled(), submitLabel("done"), onSubmit(() => void save())];
  return (
    <SwiftHost style={styles.host}>
      <BottomSheet isPresented onIsPresentedChange={(presented) => !presented && onDismiss()} fitToContents>
        <Group modifiers={[presentationDragIndicator("visible")]}>
          <VStack alignment="leading" spacing={8} modifiers={[padding({ horizontal: 16, top: 26, bottom: 16 })]}>
            {fields.map((secret, index) => (
              <HStack key={secret.name} spacing={8} modifiers={[padding({ leading: 16, trailing: 5, top: 5, bottom: 5 }), frame({ minHeight: 44 }), background(p.fill, shapes.capsule())]}>
                <SecureField autoFocus={index === 0} placeholder={secret.label} onTextChange={set(secret.name)} modifiers={[...field, accessibilityLabel(secret.label)]} />
                {index === fields.length - 1 ? (
                  <SwiftButton onPress={() => void save()} modifiers={[buttonStyle("glassProminent"), buttonBorderShape("circle"), disabled(saving || !filled), accessibilityLabel(t("Save"))]}>
                    <SwiftImage systemName="arrow.up" />
                  </SwiftButton>
                ) : null}
              </HStack>
            ))}
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
