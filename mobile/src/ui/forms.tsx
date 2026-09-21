// Inset grouped lists, the way Settings and Contacts lay out forms: a section title, rounded
// cells on the grouped background, hairline separators.

import { Button as MenuButton, Divider, HStack, Host, Image as MenuImage, Menu, Text as MenuText } from "@expo/ui/swift-ui";
import { foregroundStyle, frame, lineLimit, tint, truncationMode } from "@expo/ui/swift-ui/modifiers";
import { MenuView, type MenuAction } from "@expo/ui/community/menu";
import { type ReactNode } from "react";
import { Platform, Pressable, StyleSheet, Switch, Text, TextInput, useWindowDimensions, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export function Section({ title, footer, children, style }: { title?: string; footer?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const items = Array.isArray(children) ? children.filter(Boolean) : [children];
  return (
    <View style={[styles.section, style]}>
      {title ? <Text style={[styles.sectionTitle, { color: p.secondaryLabel }]}>{Platform.OS === "ios" ? title.toUpperCase() : title}</Text> : null}
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
/// tail when it is long. Android uses a Material 3 dropdown menu from Jetpack Compose.
function MenuAccessory({ value, title, choices }: { value: string; title: string; choices: MenuChoice[] }) {
  const p = usePalette();
  const { width } = useWindowDimensions();
  if (Platform.OS !== "ios") {
    const actions: MenuAction[] = choices.map((choice, index) => ({
      id: String(index),
      title: choice.title,
      state: choice.selected ? "on" : "off",
    }));
    return (
      <MenuView
        title={title}
        actions={actions}
        style={styles.androidMenu}
        onPressAction={({ nativeEvent }) => choices[Number(nativeEvent.event)]?.onPress()}
      >
        <View style={styles.androidMenuTrigger} accessible accessibilityRole="button" accessibilityLabel={`${title}, ${value}`}>
          <Text style={[styles.rowDetail, { color: p.secondaryLabel, maxWidth: Math.min(220, width * 0.52), textAlign: "right" }]} numberOfLines={1}>
            {value}
          </Text>
          <Symbol name="chevron.up.chevron.down" size={16} color={p.secondaryLabel} />
        </View>
      </MenuView>
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
      <Switch
        value={value}
        onValueChange={onValueChange}
        trackColor={Platform.OS === "android" ? { false: p.fill, true: p.secondaryFill } : undefined}
        thumbColor={Platform.OS === "android" ? p.tint : undefined}
      />
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
  section: { paddingHorizontal: 16, paddingTop: Platform.OS === "android" ? 24 : 20 },
  sectionTitle: { fontSize: Platform.OS === "android" ? 14 : 13, marginLeft: Platform.OS === "android" ? 0 : 16, marginBottom: 7, letterSpacing: Platform.OS === "android" ? 0 : 0.2, fontWeight: Platform.OS === "android" ? "600" : "400" },
  footer: { fontSize: 13, marginLeft: Platform.OS === "android" ? 0 : 16, marginTop: 7, lineHeight: 18 },
  group: { borderRadius: Platform.OS === "android" ? 16 : 12, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 16 },
  row: { flexDirection: "row", alignItems: "center", minHeight: Platform.OS === "android" ? 56 : 44, paddingHorizontal: 16, paddingVertical: 10, gap: 12 },
  rowMultiline: { alignItems: "flex-start" },
  rowText: { flex: 1, gap: 2 },
  rowTextWhole: { flexShrink: 0, gap: 2 },
  menu: { flex: 1, minWidth: 0 },
  androidMenu: { flex: 1, minWidth: 0, alignItems: "flex-end" },
  androidMenuTrigger: { minHeight: 36, maxWidth: "100%", flexDirection: "row", alignItems: "center", justifyContent: "flex-end", gap: 4, paddingLeft: 8 },
  rowTitle: { fontSize: Font.body },
  rowSubtitle: { fontSize: Font.small },
  rowDetail: { fontSize: Font.body, maxWidth: "55%" },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
  inputMultiline: { minHeight: 96, textAlignVertical: "top" },
});
