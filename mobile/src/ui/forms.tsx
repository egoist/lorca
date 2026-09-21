// Inset grouped lists, the way Settings and Contacts lay out forms: a section title, rounded
// cells on the grouped background, hairline separators.

import { Button as MenuButton, Divider, HStack, Host, Image as MenuImage, Menu, Text as MenuText } from "@expo/ui/swift-ui";
import { foregroundStyle, frame, lineLimit, tint, truncationMode } from "@expo/ui/swift-ui/modifiers";
import { useState, type ReactNode } from "react";
import { Modal, Platform, Pressable, ScrollView, StyleSheet, Switch, Text, TextInput, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
import { t } from "../i18n";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export function Section({ title, footer, children, style }: { title?: string; footer?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const items = Array.isArray(children) ? children.filter(Boolean) : [children];
  return (
    <View style={[styles.section, style]}>
      {title ? <Text style={[styles.sectionTitle, { color: p.secondaryLabel }]}>{title.toUpperCase()}</Text> : null}
      <View style={[styles.group, { backgroundColor: p.cell }]}>
        {items.map((child, index) => (
          <View key={index}>
            {index > 0 && <View style={[styles.separator, { backgroundColor: p.separator }]} />}
            {child}
          </View>
        ))}
      </View>
      {footer ? <Text style={[styles.footer, { color: p.secondaryLabel }]}>{footer}</Text> : null}
    </View>
  );
}

export interface MenuChoice {
  title: string;
  selected: boolean;
  onPress: () => void;
  /// A separator under this choice, as under "Automatic" in the Mac app's pop-ups.
  dividerAfter?: boolean;
}

/// A row's value that drops a native menu of choices on tap, as a pop-up button does in iOS
/// Settings: the value with the up-down chevrons, a checkmark on the choice in force. It is a
/// `Row`'s `menu`: the row keeps its title whole and the value takes what is left, cut at the
/// tail when it is long. Android uses a scrollable modal with the same choices.
function MenuAccessory({ value, title, choices }: { value: string; title: string; choices: MenuChoice[] }) {
  const p = usePalette();
  const [open, setOpen] = useState(false);
  if (Platform.OS !== "ios") {
    return (
      <>
        <Pressable onPress={() => setOpen(true)} hitSlop={8} style={styles.menu}>
          <Text style={[styles.rowDetail, { color: p.secondaryLabel, maxWidth: undefined, textAlign: "right" }]} numberOfLines={1}>
            {value}
          </Text>
        </Pressable>
        <Modal visible={open} transparent animationType="fade" onRequestClose={() => setOpen(false)}>
          <View style={styles.menuOverlay}>
            <Pressable style={styles.menuBackdrop} onPress={() => setOpen(false)} />
            <View style={[styles.menuDialog, { backgroundColor: p.cell }]}>
              <Text style={[styles.menuTitle, { color: p.label }]}>{title}</Text>
              <ScrollView style={styles.menuChoices}>
                {choices.map((choice, index) => (
                  <View key={`${choice.title}-${index}`}>
                    {index > 0 && !choices[index - 1].dividerAfter ? <View style={[styles.menuChoiceSeparator, { backgroundColor: p.separator }]} /> : null}
                    <Pressable
                      onPress={() => {
                        setOpen(false);
                        choice.onPress();
                      }}
                      style={({ pressed }) => [styles.menuChoice, pressed && { backgroundColor: p.fill }]}
                    >
                      <Text style={[styles.rowTitle, { color: p.label, flex: 1 }]}>{choice.title}</Text>
                      {choice.selected ? <Symbol name="checkmark" size={16} color={p.tint} weight="semibold" /> : null}
                    </Pressable>
                    {choice.dividerAfter ? <View style={[styles.menuDivider, { backgroundColor: p.groupedBackground }]} /> : null}
                  </View>
                ))}
              </ScrollView>
              <Pressable onPress={() => setOpen(false)} style={({ pressed }) => [styles.menuCancel, { borderTopColor: p.separator }, pressed && { backgroundColor: p.fill }]}>
                <Text style={[styles.rowTitle, { color: p.tint, fontWeight: "600" }]}>{t("Cancel")}</Text>
              </Pressable>
            </View>
          </View>
        </Modal>
      </>
    );
  }
  return (
    <Host matchContents={{ vertical: true }} style={styles.menu}>
      {/* A menu's label takes the accent color unless the menu is tinted and its parts colored. */}
      <Menu
        modifiers={[tint(p.secondaryLabel as any), frame({ maxWidth: 10000, alignment: "trailing" })]}
        label={
          <HStack spacing={4}>
            <MenuText modifiers={[foregroundStyle(p.secondaryLabel as any), lineLimit(1), truncationMode("tail")]}>{value}</MenuText>
            <MenuImage systemName="chevron.up.chevron.down" size={12} color={p.secondaryLabel} />
          </HStack>
        }
      >
        {choices.flatMap((choice) => [
          <MenuButton key={choice.title} label={choice.title} systemImage={choice.selected ? "checkmark" : undefined} onPress={choice.onPress} />,
          choice.dividerAfter ? <Divider key={`${choice.title}-divider`} /> : null,
        ])}
      </Menu>
    </Host>
  );
}

export function Row({
  title,
  detail,
  icon,
  accessory,
  menu,
  onPress,
  destructive,
  chevron,
  subtitle,
  subtitleLines = 1,
  leading,
}: {
  title: string;
  detail?: string;
  subtitle?: string;
  subtitleLines?: number;
  icon?: string;
  leading?: ReactNode;
  accessory?: ReactNode;
  /// The row's value as a native menu of choices.
  menu?: { value: string; title: string; choices: MenuChoice[] };
  onPress?: () => void;
  destructive?: boolean;
  chevron?: boolean;
}) {
  const p = usePalette();
  const color = destructive ? p.red : onPress && !chevron && !accessory ? p.tint : p.label;
  return (
    <Pressable onPress={onPress} disabled={!onPress} style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]}>
      {leading ?? (icon ? <Symbol name={icon} size={20} color={color} /> : null)}
      <View style={menu ? styles.rowTextWhole : styles.rowText}>
        <Text style={[styles.rowTitle, { color }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={[styles.rowSubtitle, { color: p.secondaryLabel }]} numberOfLines={subtitleLines} ellipsizeMode="tail">
            {subtitle}
          </Text>
        ) : null}
      </View>
      {menu ? <MenuAccessory {...menu} /> : null}
      {detail ? (
        <Text style={[styles.rowDetail, { color: p.secondaryLabel }]} numberOfLines={1}>
          {detail}
        </Text>
      ) : null}
      {accessory}
      {chevron && <Symbol name="chevron.right" size={14} color={p.tertiaryLabel} weight="semibold" />}
    </Pressable>
  );
}

export function ToggleRow({ title, value, onValueChange, icon }: { title: string; value: boolean; onValueChange: (value: boolean) => void; icon?: string }) {
  const p = usePalette();
  return (
    <View style={styles.row}>
      {icon ? <Symbol name={icon} size={20} color={p.label} /> : null}
      <Text style={[styles.rowTitle, { color: p.label, flex: 1 }]}>{title}</Text>
      <Switch value={value} onValueChange={onValueChange} />
    </View>
  );
}

export function FieldRow({ label, multiline, style, ...props }: TextInputProps & { label?: string }) {
  const p = usePalette();
  return (
    <View style={[styles.row, multiline && styles.rowMultiline]}>
      {label ? <Text style={[styles.rowTitle, { color: p.label, width: 96 }]}>{label}</Text> : null}
      <TextInput
        placeholderTextColor={p.tertiaryLabel}
        multiline={multiline}
        style={[styles.input, { color: p.label }, multiline && styles.inputMultiline, style]}
        {...props}
      />
    </View>
  );
}

export function CheckRow({ title, subtitle, checked, onPress, leading }: { title: string; subtitle?: string; checked: boolean; onPress: () => void; leading?: ReactNode }) {
  const p = usePalette();
  return (
    <Pressable onPress={onPress} style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]}>
      {leading}
      <View style={styles.rowText}>
        <Text style={[styles.rowTitle, { color: p.label }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={[styles.rowSubtitle, { color: p.secondaryLabel }]} numberOfLines={1}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {checked ? <Symbol name="checkmark" size={16} color={p.tint} weight="semibold" /> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  section: { paddingHorizontal: 16, paddingTop: 20 },
  sectionTitle: { fontSize: 13, marginLeft: 16, marginBottom: 7, letterSpacing: 0.2 },
  footer: { fontSize: 13, marginLeft: 16, marginTop: 7, lineHeight: 18 },
  group: { borderRadius: 12, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 16 },
  row: { flexDirection: "row", alignItems: "center", minHeight: 44, paddingHorizontal: 16, paddingVertical: 10, gap: 12 },
  rowMultiline: { alignItems: "flex-start" },
  rowText: { flex: 1, gap: 2 },
  rowTextWhole: { flexShrink: 0, gap: 2 },
  menu: { flex: 1, minWidth: 0 },
  menuOverlay: { flex: 1, justifyContent: "center", paddingHorizontal: 28 },
  menuBackdrop: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0, backgroundColor: "rgba(0,0,0,0.35)" },
  menuDialog: { borderRadius: 16, overflow: "hidden", maxHeight: "75%" },
  menuTitle: { fontSize: Font.body, fontWeight: "700", paddingHorizontal: 16, paddingTop: 16, paddingBottom: 10 },
  menuChoices: { flexShrink: 1 },
  menuChoice: { minHeight: 48, paddingHorizontal: 16, paddingVertical: 12, flexDirection: "row", alignItems: "center", gap: 12 },
  menuChoiceSeparator: { height: StyleSheet.hairlineWidth, marginLeft: 16 },
  menuDivider: { height: 8 },
  menuCancel: { minHeight: 48, borderTopWidth: StyleSheet.hairlineWidth, alignItems: "center", justifyContent: "center" },
  rowTitle: { fontSize: Font.body },
  rowSubtitle: { fontSize: Font.small },
  rowDetail: { fontSize: Font.body, maxWidth: "55%" },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
  inputMultiline: { minHeight: 96, textAlignVertical: "top" },
});
