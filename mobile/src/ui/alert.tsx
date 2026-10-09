// Alerts as each platform draws them: UIKit's alert on iOS (React Native's Alert), and on Android a
// Material 3 dialog from Compose, which React Native's AppCompat dialog is not (square corners,
// AppCompat buttons, no red for a destructive action). Same arguments as `Alert.alert`.

import { AlertDialog, Host, OutlinedTextField, Row, Text as ComposeText, TextButton, type TextFieldRef } from "@expo/ui/jetpack-compose";
import { fillMaxWidth } from "@expo/ui/jetpack-compose/modifiers";
import { useEffect, useRef, useState } from "react";
import { Alert, Platform, StyleSheet, type AlertButton, type AlertOptions } from "react-native";
import { create } from "zustand";
import { t } from "../i18n";
import { usePalette } from "./theme";

interface Pending {
  title: string;
  message?: string;
  buttons: AlertButton[];
  cancelable: boolean;
  onDismiss?: () => void;
  /// A prompt's field: its first text, its placeholder, and what the confirming button does with it.
  field?: { value: string; placeholder?: string; done: (text: string) => void };
}

const useAlerts = create<{ queue: Pending[] }>(() => ({ queue: [] }));

export function alert(title: string, message?: string, buttons?: AlertButton[], options?: AlertOptions) {
  if (Platform.OS !== "android") return Alert.alert(title, message, buttons, options);
  const pending: Pending = {
    title,
    message,
    buttons: buttons?.length ? buttons : [{ text: t("OK") }],
    cancelable: options?.cancelable ?? true,
    onDismiss: options?.onDismiss,
  };
  useAlerts.setState((s) => ({ queue: [...s.queue, pending] }));
}

/// Asks for one line of text: UIKit's alert with a field on iOS, a Material dialog with one on
/// Android. `done` gets the text when `confirm` is pressed.
export function prompt(title: string, options: { message?: string; value?: string; placeholder?: string; confirm: string; done: (text: string) => void }) {
  const { message, value = "", placeholder, confirm, done } = options;
  if (Platform.OS !== "android") {
    Alert.prompt(title, message, [{ text: t("Cancel"), style: "cancel" }, { text: confirm, onPress: (text?: string) => done(text ?? "") }], "plain-text", value);
    return;
  }
  const pending: Pending = { title, message, buttons: [{ text: t("Cancel"), style: "cancel" }, { text: confirm }], cancelable: true, field: { value, placeholder, done } };
  useAlerts.setState((s) => ({ queue: [...s.queue, pending] }));
}

/// The dialog on screen, one at a time. Mounted once at the root on Android.
export function AlertHost() {
  const current = useAlerts((s) => s.queue[0]);
  const p = usePalette();
  const [text, setText] = useState("");
  const field = useRef<TextFieldRef>(null);
  // A prompt's field starts with its text, once per dialog.
  useEffect(() => {
    if (!current?.field) return;
    setText(current.field.value);
    if (current.field.value) void field.current?.setText(current.field.value);
  }, [current]);
  if (Platform.OS !== "android" || !current) return null;
  const close = (button?: AlertButton) => {
    useAlerts.setState((s) => ({ queue: s.queue.slice(1) }));
    if (button && current.field && button.style !== "cancel") current.field.done(text);
    if (button) button.onPress?.();
    else current.onDismiss?.();
  };
  // Material puts the confirming action last, the dismissive one before it, and any third at
  // the start of the row.
  const cancel = current.buttons.find((button) => button.style === "cancel");
  const others = current.buttons.filter((button) => button !== cancel);
  const confirm = others[others.length - 1] ?? cancel!;
  const before = [...others.slice(0, -1), ...(cancel && confirm !== cancel ? [cancel] : [])];
  const color = (button: AlertButton) => ({ contentColor: (button.style === "destructive" ? p.red : p.tint) as string });
  return (
    <Host style={StyleSheet.absoluteFill} pointerEvents="box-none">
      <AlertDialog
        onDismissRequest={() => {
          if (current.cancelable) close(cancel);
        }}
        colors={{ containerColor: p.cell as string, titleContentColor: p.label as string, textContentColor: p.secondaryLabel as string }}
      >
        <AlertDialog.Title>
          <ComposeText style={{ typography: "headlineSmall" }}>{current.title}</ComposeText>
        </AlertDialog.Title>
        {current.field ? (
          <AlertDialog.Text>
            <OutlinedTextField ref={field} autoFocus singleLine onValueChange={setText} modifiers={[fillMaxWidth()]}>
              {current.field.placeholder ? (
                <OutlinedTextField.Placeholder>
                  <ComposeText>{current.field.placeholder}</ComposeText>
                </OutlinedTextField.Placeholder>
              ) : null}
            </OutlinedTextField>
          </AlertDialog.Text>
        ) : current.message ? (
          <AlertDialog.Text>
            <ComposeText style={{ typography: "bodyMedium" }}>{current.message}</ComposeText>
          </AlertDialog.Text>
        ) : null}
        <AlertDialog.ConfirmButton>
          <TextButton onClick={() => close(confirm)} colors={color(confirm)}>
            <ComposeText>{confirm.text ?? t("OK")}</ComposeText>
          </TextButton>
        </AlertDialog.ConfirmButton>
        {before.length ? (
          <AlertDialog.DismissButton>
            <Row>
              {before.map((button, index) => (
                <TextButton key={index} onClick={() => close(button)} colors={color(button)}>
                  <ComposeText>{button.text ?? ""}</ComposeText>
                </TextButton>
              ))}
            </Row>
          </AlertDialog.DismissButton>
        ) : null}
      </AlertDialog>
    </Host>
  );
}
