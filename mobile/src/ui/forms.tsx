// Inset grouped lists, the way Settings and Contacts lay out forms: a section title, rounded
// cells on the grouped background, hairline separators.

import { Button as MenuButton, Divider, HStack, Host, Image as MenuImage, Menu, Text as MenuText } from "@expo/ui/swift-ui";
import { contentShape, font, foregroundStyle, frame, lineLimit, menuOrder, padding, shapes, tint, truncationMode } from "@expo/ui/swift-ui/modifiers";
import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import { Checkbox, Host as ComposeHost, RadioButton, Switch as ComposeSwitch } from "@expo/ui/jetpack-compose";
import { Children, isValidElement, useRef, type ReactNode } from "react";
import { Platform, StyleSheet, Switch, Text, TextInput, useWindowDimensions, View, type StyleProp, type TextInputProps, type ViewStyle } from "react-native";
import { Pressable } from "./Pressable";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export function Section({ title, footer, children, style }: { title?: string; footer?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  // Rows from a `{list.map(…)}` beside other rows arrive as one nested array; flattened, each
  // row gets its own separator. `toArray` drops empty children and keys every row by its place
  // and its own key, so a row keeps its cell when the rows around it come and go.
  const rows = Children.toArray(children).filter(isValidElement);
  return (
    <View style={[styles.section, style]}>
      {/* Material's list subheaders take the primary color; iOS's are secondary and uppercase. */}
      {title ? <Text style={[styles.sectionTitle, { color: Platform.OS === "ios" ? p.secondaryLabel : p.tint }]}>{Platform.OS === "ios" ? title.toUpperCase() : title}</Text> : null}
      <View style={[styles.group, { backgroundColor: p.cell }]}>
        {rows.map((row, index) => (
          <View key={row.key}>
            {index > 0 && <View style={[styles.separator, { backgroundColor: p.separator }]} />}
            {row}
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
        {/* Keyed by place: two choices can share a title, as models a server names alike do. */}
        {choices.flatMap((choice, index) => [
          <MenuButton key={`choice-${index}`} label={choice.title} systemImage={choice.selected ? "checkmark" : undefined} onPress={choice.onPress} />,
          choice.dividerAfter ? <Divider key={`divider-${index}`} /> : null,
        ])}
      </Menu>
    </Host>
  );
}

/// An action row whose tap drops a native menu of choices, for an action that comes in kinds:
/// the title in the tint color, after its symbol when it has one, as an action row's is. On iOS
/// the row is the SwiftUI menu's label, drawn to match a `Row`; Android opens a Material dropdown
/// from the row.
export function MenuRow({ title, icon, choices }: { title: string; icon?: string; choices: MenuChoice[] }) {
  const p = usePalette();
  const menu = useRef<MenuComponentRef>(null);
  if (Platform.OS !== "ios") {
    // Groups between dividers become inline sections, which Material draws with a divider above
    // and below: the first group and the last stay plain so no divider ends the menu.
    const groups: { choice: MenuChoice; index: number }[][] = [[]];
    choices.forEach((choice, index) => {
      groups[groups.length - 1].push({ choice, index });
      if (choice.dividerAfter && index < choices.length - 1) groups.push([]);
    });
    const action = ({ choice, index }: { choice: MenuChoice; index: number }): MenuAction => ({ id: String(index), title: choice.title, state: choice.selected ? "on" : undefined });
    const actions = groups.flatMap((group, at): MenuAction[] =>
      at === 0 || (at === groups.length - 1 && groups.length > 2) ? group.map(action) : [{ id: `group-${at}`, title: "", displayInline: true, subactions: group.map(action) }],
    );
    return (
      <MenuView ref={menu} title={title} actions={actions} shouldOpenOnLongPress onPressAction={({ nativeEvent }) => choices[Number(nativeEvent.event)]?.onPress()}>
        <Row title={title} icon={icon} onPress={() => menu.current?.show()} />
      </MenuView>
    );
  }
  return (
    <Host matchContents={{ vertical: true }} style={styles.menuRow}>
      <Menu
        modifiers={[menuOrder("fixed"), tint(p.tint as any)]}
        label={
          <HStack spacing={12} modifiers={[frame({ maxWidth: 10000, minHeight: 44, alignment: "leading" }), padding({ horizontal: 16 }), contentShape(shapes.rectangle())]}>
            {icon ? <MenuImage systemName={icon as any} size={17} color={p.tint} modifiers={[frame({ width: 20 })]} /> : null}
            <MenuText modifiers={[font({ size: Font.body }), foregroundStyle(p.tint as any), lineLimit(1)]}>{title}</MenuText>
          </HStack>
        }
      >
        {choices.flatMap((choice, index) => [
          <MenuButton key={`choice-${index}`} label={choice.title} systemImage={choice.selected ? "checkmark" : undefined} onPress={choice.onPress} />,
          choice.dividerAfter ? <Divider key={`divider-${index}`} /> : null,
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
  titleLines = 1,
  leading,
  action,
  disabled,
  onLongPress,
}: {
  title: string;
  detail?: string;
  subtitle?: string;
  subtitleLines?: number;
  /// How many lines a long title may wrap to before it is cut short.
  titleLines?: number;
  icon?: string;
  leading?: ReactNode;
  accessory?: ReactNode;
  /// The row's value as a native menu of choices.
  menu?: { value: string; title: string; choices: MenuChoice[] };
  onPress?: () => void;
  destructive?: boolean;
  chevron?: boolean;
  /// Draws the title in the tint color, as an action is; by default a row that only acts on a tap.
  action?: boolean;
  /// An action that can't be taken yet: its title in the tertiary color, and no tap.
  disabled?: boolean;
  onLongPress?: () => void;
}) {
  const p = usePalette();
  const color = disabled ? p.tertiaryLabel : destructive ? p.red : (action ?? (onPress && !chevron && !accessory)) ? p.tint : p.label;
  return (
    <Pressable
      onPress={onPress}
      onLongPress={onLongPress}
      disabled={disabled || (!onPress && !onLongPress)}
      accessibilityState={disabled ? { disabled } : undefined}
      style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]}
    >
      {leading ?? (icon ? <Symbol name={icon} size={20} color={color} /> : null)}
      <View style={menu ? styles.rowTextWhole : styles.rowText}>
        <Text style={[styles.rowTitle, { color }]} numberOfLines={titleLines}>
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
      {/* A disclosure chevron is iOS's; a Material list item that opens a screen has none. */}
      {chevron && Platform.OS === "ios" && <Symbol name="chevron.right" size={14} color={p.tertiaryLabel} weight="semibold" />}
    </Pressable>
  );
}

/// A setting that is on or off: UIKit's switch on iOS; on Android Material 3's (Compose), with the
/// whole row toggling it, as a Material list item with a switch does.
export function ToggleRow({ title, value, onValueChange, icon }: { title: string; value: boolean; onValueChange: (value: boolean) => void; icon?: string }) {
  const p = usePalette();
  if (Platform.OS === "android")
    return (
      <Pressable onPress={() => onValueChange(!value)} style={styles.row} accessibilityRole="switch" accessibilityState={{ checked: value }} accessibilityLabel={title}>
        {icon ? <Symbol name={icon} size={20} color={p.label} /> : null}
        <Text style={[styles.rowTitle, { color: p.label, flex: 1 }]}>{title}</Text>
        <ComposeHost matchContents>
          <ComposeSwitch value={value} onCheckedChange={onValueChange} />
        </ComposeHost>
      </Pressable>
    );
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
      {/* One line in the label column: a longer label ("API 基础地址") shrinks a little rather than wrap. */}
      {label ? (
        <Text style={[styles.rowTitle, { color: p.label, width: 96 }]} numberOfLines={1} adjustsFontSizeToFit minimumFontScale={0.8}>
          {label}
        </Text>
      ) : null}
      <TextInput
        placeholderTextColor={p.tertiaryLabel}
        multiline={multiline}
        style={[styles.input, { color: p.label }, multiline && styles.inputMultiline, style]}
        {...props}
      />
    </View>
  );
}

/// One choice of several: a checkmark after it on iOS, Material's radio button before it on
/// Android, or its checkbox where several can be picked (`multiple`). A row that acts at once rather
/// than marking a choice (`indicator={false}`) shows neither. A choice that can't be made right now
/// (`disabled`) is dimmed and takes no tap.
export function CheckRow({ title, subtitle, checked, onPress, leading, multiple, indicator = true, disabled }: { title: string; subtitle?: string; checked: boolean; onPress: () => void; leading?: ReactNode; multiple?: boolean; indicator?: boolean; disabled?: boolean }) {
  const p = usePalette();
  const android = Platform.OS === "android";
  return (
    <Pressable onPress={onPress} disabled={disabled} style={({ pressed }) => [styles.row, pressed && { backgroundColor: p.fill }]} accessibilityRole={multiple ? "checkbox" : "radio"} accessibilityState={{ checked, disabled }}>
      {android && indicator ? (
        <ComposeHost matchContents>
          {multiple ? <Checkbox value={checked} enabled={!disabled} onCheckedChange={onPress} /> : <RadioButton selected={checked} onClick={onPress} />}
        </ComposeHost>
      ) : null}
      {leading}
      <View style={styles.rowText}>
        <Text style={[styles.rowTitle, { color: disabled ? p.tertiaryLabel : p.label }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={[styles.rowSubtitle, { color: p.secondaryLabel }]} numberOfLines={1}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {checked && !android ? <Symbol name="checkmark" size={16} color={p.tint} weight="semibold" /> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  section: { paddingHorizontal: 16, paddingTop: Platform.OS === "android" ? 24 : 20 },
  sectionTitle: { fontSize: Platform.OS === "android" ? 14 : 13, marginLeft: 16, marginBottom: 7, letterSpacing: Platform.OS === "android" ? 0.1 : 0.2, fontWeight: Platform.OS === "android" ? "500" : "400" },
  footer: { fontSize: 13, marginHorizontal: 16, marginTop: 7, lineHeight: 18 },
  group: { borderRadius: Platform.OS === "android" ? 16 : 12, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 16 },
  row: { flexDirection: "row", alignItems: "center", minHeight: Platform.OS === "android" ? 56 : 44, paddingHorizontal: 16, paddingVertical: 10, gap: 12 },
  rowMultiline: { alignItems: "flex-start" },
  rowText: { flex: 1, gap: 2 },
  rowTextWhole: { flexShrink: 0, gap: 2 },
  menu: { flex: 1, minWidth: 0 },
  menuRow: { alignSelf: "stretch" },
  androidMenu: { flex: 1, minWidth: 0, alignItems: "flex-end" },
  androidMenuTrigger: { minHeight: 36, maxWidth: "100%", flexDirection: "row", alignItems: "center", justifyContent: "flex-end", gap: 4, paddingLeft: 8 },
  rowTitle: { fontSize: Font.body },
  rowSubtitle: { fontSize: Font.small },
  rowDetail: { fontSize: Font.body, maxWidth: "55%" },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
  inputMultiline: { minHeight: 96, textAlignVertical: "top" },
});
