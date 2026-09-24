// The chat list's search field wherever the native bar cannot sit at the foot: a capsule
// floating under the list, level with the composer, riding the keyboard. UIKit moves a search
// bar into the bottom toolbar only on an iPhone. On Android the search action is an icon in the
// header, an item of the activity's one action bar, which react-native-screens hands to
// whichever stack updated its header last, so a sidebar's item vanishes once the pane beside it
// opens a chat.

import { useRef } from "react";
import { Pressable, StyleSheet, TextInput } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { t, useLanguage } from "../i18n";
import { Surface } from "./Composer";
import { KeyboardFoot } from "./KeyboardFoot";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

/// The composer's pill, so the two line up across the panes.
const HEIGHT = 46;
const MARGIN = 16;

function useFoot(): number {
  return Math.max(useSafeAreaInsets().bottom, 8) + 8;
}

/// The room the list leaves under its last row for the field.
export function useSidebarSearchInset(): number {
  return HEIGHT + useFoot();
}

export function SidebarSearch({ value, placeholder, onChangeText }: { value: string; placeholder: string; onChangeText: (value: string) => void }) {
  useLanguage();
  const p = usePalette();
  const input = useRef<TextInput>(null);
  const bottom = useFoot();
  return (
    <KeyboardFoot style={[styles.wrap, { paddingBottom: bottom }]} tuck={bottom - 8}>
      <Surface style={styles.field} tint={p.cell} edge={p.dark ? "rgba(255,255,255,0.14)" : "rgba(0,0,0,0.1)"} onPress={() => input.current?.focus()}>
        <Symbol name="magnifyingglass" size={17} color={p.secondaryLabel} />
        <TextInput
          ref={input}
          style={[styles.input, { color: p.label }]}
          // Uncontrolled: the list re-renders on every keystroke, and a controlled field would
          // drop the characters typed meanwhile.
          onChangeText={onChangeText}
          placeholder={placeholder}
          placeholderTextColor={p.secondaryLabel as any}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          clearButtonMode="never"
        />
        {value ? (
          <Pressable
            onPress={() => {
              input.current?.clear();
              onChangeText("");
            }}
            hitSlop={10} accessibilityRole="button" accessibilityLabel={t("Clear")}>
            <Symbol name="xmark.circle.fill" size={17} color={p.secondaryLabel} />
          </Pressable>
        ) : null}
      </Surface>
    </KeyboardFoot>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "absolute", left: 0, right: 0, bottom: 0, paddingHorizontal: MARGIN },
  field: { height: HEIGHT, borderRadius: HEIGHT / 2, flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 14 },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
});
