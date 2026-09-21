import { Stack } from "expo-router";
import { useMemo } from "react";
import { Platform, type ImageSourcePropType } from "react-native";
import { usePalette } from "./theme";

export const AndroidIcons = {
  settings: require("../../assets/material/settings.xml") as ImageSourcePropType,
  add: require("../../assets/material/add.xml") as ImageSourcePropType,
  close: require("../../assets/material/close.xml") as ImageSourcePropType,
  check: require("../../assets/material/check.xml") as ImageSourcePropType,
  group: require("../../assets/material/group.xml") as ImageSourcePropType,
  personAdd: require("../../assets/material/person_add.xml") as ImageSourcePropType,
  pin: require("../../assets/material/push_pin.xml") as ImageSourcePropType,
  read: require("../../assets/material/done_all.xml") as ImageSourcePropType,
  delete: require("../../assets/material/delete.xml") as ImageSourcePropType,
  photos: require("../../assets/material/photo_library.xml") as ImageSourcePropType,
  camera: require("../../assets/material/camera.xml") as ImageSourcePropType,
  folder: require("../../assets/material/folder.xml") as ImageSourcePropType,
} as const;

/** The bar every stack shares: tinted on iOS, Material's plain label color on Android. */
export function useStackScreenOptions() {
  const p = usePalette();
  return useMemo(
    () => ({
      headerTintColor: (Platform.OS === "android" ? p.label : p.tint) as any,
      headerTitleStyle: { color: p.label as any },
      headerTitleAlign: Platform.OS === "android" ? ("left" as const) : undefined,
      headerBackButtonDisplayMode: "minimal" as const,
      contentStyle: { backgroundColor: p.background },
    }),
    [p.dark],
  );
}

/** Cancel + commit actions for a presented editor. Android uses Material icon actions. */
export function FormToolbar({
  cancelLabel,
  saveLabel,
  saveDisabled,
  onCancel,
  onSave,
}: {
  cancelLabel: string;
  saveLabel: string;
  saveDisabled?: boolean;
  onCancel: () => void;
  onSave: () => void;
}) {
  if (Platform.OS === "android") {
    return (
      <>
        <Stack.Toolbar placement="left">
          <Stack.Toolbar.Button icon={AndroidIcons.close} accessibilityLabel={cancelLabel} onPress={onCancel} />
        </Stack.Toolbar>
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon={AndroidIcons.check} accessibilityLabel={saveLabel} disabled={saveDisabled} onPress={onSave} />
        </Stack.Toolbar>
      </>
    );
  }
  return (
    <>
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button onPress={onCancel}>{cancelLabel}</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" disabled={saveDisabled} onPress={onSave}>
          {saveLabel}
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
    </>
  );
}

/** Close a screen whose changes already apply as they are made. */
export function CloseToolbar({ label, onClose }: { label: string; onClose: () => void }) {
  if (Platform.OS === "android") {
    return (
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button icon={AndroidIcons.close} accessibilityLabel={label} onPress={onClose} />
      </Stack.Toolbar>
    );
  }
  return (
    <Stack.Toolbar placement="right">
      <Stack.Toolbar.Button variant="done" onPress={onClose}>
        {label}
      </Stack.Toolbar.Button>
    </Stack.Toolbar>
  );
}

/** Commit action for an editor pushed inside an existing stack. */
export function SaveToolbar({ label, disabled, onSave }: { label: string; disabled?: boolean; onSave: () => void }) {
  return (
    <Stack.Toolbar placement="right">
      {Platform.OS === "android" ? (
        <Stack.Toolbar.Button icon={AndroidIcons.check} accessibilityLabel={label} disabled={disabled} onPress={onSave} />
      ) : (
        <Stack.Toolbar.Button variant="done" disabled={disabled} onPress={onSave}>
          {label}
        </Stack.Toolbar.Button>
      )}
    </Stack.Toolbar>
  );
}

/** Close icon used by a full-screen media viewer on both platforms. */
export function IconCloseToolbar({ label, onClose }: { label: string; onClose: () => void }) {
  return (
    <Stack.Toolbar placement="left">
      <Stack.Toolbar.Button icon={Platform.OS === "android" ? AndroidIcons.close : "xmark"} accessibilityLabel={label} onPress={onClose} />
    </Stack.Toolbar>
  );
}
