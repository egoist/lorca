// The custom provider form's models, slid in from its Models row inside the same sheet: the ones
// saved or added by hand and the ones the server lists, each picked or not. The search filters
// them, and offers to add an id the server does not list. Changes land in the form's draft.

import { FlashList } from "@shopify/flash-list";
import { Stack } from "expo-router";
import { useRef, useState } from "react";
import { ActivityIndicator, Platform, Pressable, StyleSheet, Text, View } from "react-native";
import type { SearchBarCommands } from "react-native-screens";
import { contextWindowLabel, filterModelRows, modelIdToAdd, modelLabel, modelListingNote, type ModelRow } from "../../src/core/model";
import { t, useLanguage } from "../../src/i18n";
import { addModel, toggleModel, useModelDraft } from "../../src/ui/modelDraft";
import { Symbol } from "../../src/ui/Symbol";
import { Font, usePalette } from "../../src/ui/theme";

type Item = { kind: "add"; id: string } | { kind: "model"; row: ModelRow };

/// A filtered list starts at its top, as the chat list does: FlashList would otherwise keep the
/// row that was in view in place. The list also leaves the keyboard's inset alone: adjusting it
/// put the list's top, the row that adds the typed id, under the bar.
const KEEP_OFFSET = { disabled: true };

export default function CustomModelsScreen() {
  useLanguage();
  const p = usePalette();
  const rows = useModelDraft((s) => s.rows);
  const listing = useModelDraft((s) => s.listing);
  const search = useRef<SearchBarCommands>(null);
  const [query, setQuery] = useState("");
  const addId = modelIdToAdd(rows, query);
  const items: Item[] = [...(addId ? [{ kind: "add" as const, id: addId }] : []), ...filterModelRows(rows, query).map((row) => ({ kind: "model" as const, row }))];
  const note = modelListingNote(listing);
  // A column for the image mark when any model takes images, so the numbers line up.
  const images = rows.some((row) => row.images);

  /// The typed id, picked, at the top; the search starts over.
  function add(id: string) {
    addModel(id);
    setQuery("");
    search.current?.clearText();
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Models") }} />
      <Stack.SearchBar
        ref={search}
        placeholder={t("Search or add a model ID")}
        onChangeText={(e) => setQuery(e.nativeEvent.text)}
        onCancelButtonPress={() => setQuery("")}
        onSearchButtonPress={(e) => {
          const id = modelIdToAdd(useModelDraft.getState().rows, e.nativeEvent.text);
          if (id) add(id);
        }}
        autoCapitalize="none"
        hideWhenScrolling={false}
        // The title and the way back stay while searching.
        hideNavigationBar={false}
        // The list stays live under the search: a tap picks a model rather than ending the search.
        obscureBackground={false}
      />
      <FlashList
        data={items}
        extraData={items.length}
        keyExtractor={(item) => (item.kind === "add" ? "add" : `model:${item.row.id}`)}
        contentInsetAdjustmentBehavior="automatic"
        maintainVisibleContentPosition={KEEP_OFFSET}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={styles.content}
        renderItem={({ item, index }) => (
          <ModelCell item={item} first={index === 0} last={index === items.length - 1} images={images} onAdd={add} />
        )}
        ListFooterComponent={items.length && note ? <Text style={[styles.footer, { color: p.secondaryLabel }]}>{note}</Text> : null}
        ListEmptyComponent={
          <View style={styles.empty}>
            {listing.state === "loading" ? <ActivityIndicator /> : null}
            {note ? <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{note}</Text> : null}
          </View>
        }
      />
    </>
  );
}

/// One cell of the inset group the list draws: the row that adds the typed id, or a model with
/// its id under its name, its context window, whether it takes images, and a check when picked.
function ModelCell({ item, first, last, images, onAdd }: { item: Item; first: boolean; last: boolean; images: boolean; onAdd: (id: string) => void }) {
  const p = usePalette();
  const shape = [styles.cell, first && styles.first, last && styles.last];
  const separator = first ? null : <View style={[styles.separator, { backgroundColor: p.separator }]} />;
  if (item.kind === "add") {
    return (
      <Pressable onPress={() => onAdd(item.id)} accessibilityRole="button" style={({ pressed }) => [...shape, { backgroundColor: pressed ? p.fill : p.cell }]}>
        {separator}
        <Symbol name="plus" size={20} color={p.tint} />
        <Text style={[styles.title, styles.text, { color: p.tint }]} numberOfLines={1}>
          {t("Add “{id}”", { id: item.id })}
        </Text>
      </Pressable>
    );
  }
  const { row } = item;
  const label = modelLabel(row);
  return (
    <Pressable
      onPress={() => toggleModel(row.id)}
      accessibilityRole="button"
      accessibilityState={{ selected: row.selected }}
      style={({ pressed }) => [...shape, { backgroundColor: pressed ? p.fill : p.cell }]}
    >
      {separator}
      <View style={styles.text}>
        <Text style={[styles.title, { color: p.label }]} numberOfLines={1}>
          {label}
        </Text>
        {label !== row.id ? (
          <Text style={[styles.subtitle, { color: p.secondaryLabel }]} numberOfLines={1}>
            {row.id}
          </Text>
        ) : null}
      </View>
      {row.context_window ? <Text style={[styles.detail, { color: p.secondaryLabel }]}>{contextWindowLabel(row.context_window)}</Text> : null}
      {images ? (
        <View style={styles.slot} accessible={!!row.images} accessibilityLabel={row.images ? t("Images") : undefined}>
          {row.images ? <Symbol name="photo" size={15} color={p.secondaryLabel} /> : null}
        </View>
      ) : null}
      <View style={styles.slot}>{row.selected ? <Symbol name="checkmark" size={16} color={p.tint} weight="semibold" /> : null}</View>
    </Pressable>
  );
}

const RADIUS = Platform.OS === "android" ? 16 : 12;

const styles = StyleSheet.create({
  content: { paddingTop: Platform.OS === "android" ? 24 : 20, paddingBottom: 40 },
  cell: { flexDirection: "row", alignItems: "center", minHeight: Platform.OS === "android" ? 56 : 44, marginHorizontal: 16, paddingHorizontal: 16, paddingVertical: 10, gap: 12 },
  first: { borderTopLeftRadius: RADIUS, borderTopRightRadius: RADIUS },
  last: { borderBottomLeftRadius: RADIUS, borderBottomRightRadius: RADIUS },
  separator: { position: "absolute", top: 0, left: 16, right: 0, height: StyleSheet.hairlineWidth },
  text: { flex: 1, gap: 2 },
  title: { fontSize: Font.body },
  subtitle: { fontSize: Font.small },
  detail: { fontSize: Font.body },
  slot: { width: 18, alignItems: "center" },
  footer: { fontSize: 13, lineHeight: 18, marginHorizontal: Platform.OS === "android" ? 16 : 32, marginTop: 7 },
  empty: { alignItems: "center", gap: 12, paddingHorizontal: 32, paddingTop: 48 },
  emptyText: { fontSize: 15, lineHeight: 20, textAlign: "center" },
});
