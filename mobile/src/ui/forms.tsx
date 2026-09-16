// Inset grouped lists, the way Settings and Contacts lay out forms: a section title, rounded
// cells on the grouped background, hairline separators.

import type { ReactNode } from "react";
import { Pressable, StyleSheet, Switch, Text, TextInput, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
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

export function Row({
  title,
  detail,
  icon,
  accessory,
  onPress,
  destructive,
  chevron,
  subtitle,
  leading,
}: {
  title: string;
  detail?: string;
  subtitle?: string;
  icon?: string;
  leading?: ReactNode;
  accessory?: ReactNode;
  onPress?: () => void;
  destructive?: boolean;
  chevron?: boolean;
}) {
  const p = usePalette();
  const color = destructive ? p.red : onPress && !chevron && !accessory ? p.tint : p.label;
  return (
    <Pressable onPress={onPress} disabled={!onPress} style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]}>
      {leading ?? (icon ? <Symbol name={icon} size={20} color={color} /> : null)}
      <View style={styles.rowText}>
        <Text style={[styles.rowTitle, { color }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={[styles.rowSubtitle, { color: p.secondaryLabel }]} numberOfLines={1}>
            {subtitle}
          </Text>
        ) : null}
      </View>
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
  rowTitle: { fontSize: Font.body },
  rowSubtitle: { fontSize: Font.small },
  rowDetail: { fontSize: Font.body, maxWidth: "55%" },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
  inputMultiline: { minHeight: 96, textAlignVertical: "top" },
});
