import { expect, mock, test } from "bun:test";
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";

// `Section` lays a separator between every two rows, however the screen passes them: one by
// one, as a `{list.map(…)}` among other rows, or a list that is empty. The native pieces are
// plain tags here, and a section reads back as its rows' titles with "—" for each separator.
mock.module("react-native", () => ({
  Platform: { OS: "ios" },
  StyleSheet: { create: <T>(styles: T) => styles, hairlineWidth: 0.5 },
  View: "View",
  Text: "Text",
  Pressable: "Pressable",
  Switch: "Switch",
  TextInput: "TextInput",
  useWindowDimensions: () => ({ width: 402, height: 874 }),
}));
mock.module("@expo/ui/swift-ui", () => ({ Button: "Button", Divider: "Divider", HStack: "HStack", Host: "Host", Image: "Image", Menu: "Menu", Text: "Text" }));
mock.module("@expo/ui/swift-ui/modifiers", () => ({
  contentShape: () => ({}),
  font: () => ({}),
  foregroundStyle: () => ({}),
  frame: () => ({}),
  lineLimit: () => ({}),
  menuOrder: () => ({}),
  padding: () => ({}),
  shapes: {},
  tint: () => ({}),
  truncationMode: () => ({}),
}));
mock.module("@expo/ui/community/menu", () => ({ MenuView: "MenuView" }));
mock.module("./Symbol", () => ({ Symbol: "Symbol" }));
mock.module("./theme", () => ({ Font: { body: 17, small: 15 }, usePalette: () => ({ cell: "cell", separator: "separator", secondaryLabel: "secondaryLabel" }) }));

const { Section } = await import("./forms");

const row = (title: string) => createElement("row", { key: title, title });

/// The section's rows top to bottom, "—" for a separator; the title and footer are left out.
function layout(node: ReactNode): string[] {
  if (!isValidElement<{ title?: string; children?: ReactNode }>(node)) return [];
  if (node.type === "row") return [node.props.title!];
  if (node.type === "Text") return [];
  if (node.type === "View" && node.props.children === undefined) return ["—"];
  return Children.toArray(node.props.children).flatMap(layout);
}

/// The keys of the cells that hold the rows.
function cellKeys(section: ReactElement<{ children: ReactNode[] }>): (string | null)[] {
  const group = section.props.children.find((child) => isValidElement(child) && child.type === "View") as ReactElement<{ children: ReactElement[] }>;
  return group.props.children.map((cell) => cell.key);
}

test("a mapped list after a row gets a separator before each of its rows", () => {
  // New Bot's Model and Thinking: Default, then the provider's choices.
  expect(layout(Section({ title: "Model", children: [row("Default"), ["Flash", "Pro"].map(row)] }))).toEqual(["Default", "—", "Flash", "—", "Pro"]);
  expect(layout(Section({ title: "Thinking", children: [row("Default"), ["High", "Max"].map(row)] }))).toEqual(["Default", "—", "High", "—", "Max"]);
});

test("a mapped list between rows has separators on both sides and none doubled when empty", () => {
  // Auto-review: the switch, the rules, then Add rule.
  const rules = (texts: string[]) => layout(Section({ title: "Auto-review", children: [row("Check actions"), texts.map(row), row("Add rule…")] }));
  expect(rules(["Run tests", "Push to main"])).toEqual(["Check actions", "—", "Run tests", "—", "Push to main", "—", "Add rule…"]);
  expect(rules([])).toEqual(["Check actions", "—", "Add rule…"]);
});

test("a mapped list first, with a row after it only some of the time", () => {
  // A group's members, then Add Bot while the group has room.
  const members = ["Chef", "Scout", "Tea"].map(row);
  expect(layout(Section({ title: "Members", children: [members, row("Add Bot")] }))).toEqual(["Chef", "—", "Scout", "—", "Tea", "—", "Add Bot"]);
  expect(layout(Section({ title: "Members", children: [members, null] }))).toEqual(["Chef", "—", "Scout", "—", "Tea"]);
});

test("a section of one row, or of none, has no separator", () => {
  expect(layout(Section({ children: row("Delete Chat") }))).toEqual(["Delete Chat"]);
  expect(layout(Section({ children: [] }))).toEqual([]);
});

test("each row keeps its cell when the rows around it come and go", () => {
  const keys = (titles: string[]) => cellKeys(Section({ children: [row("Default"), titles.map(row)] }));
  const [defaultKey, flash, plus, pro] = keys(["Flash", "Plus", "Pro"]);
  expect(new Set([defaultKey, flash, plus, pro]).size).toBe(4);
  expect(keys(["Flash", "Pro"])).toEqual([defaultKey, flash, pro]);
  expect(keys(["Pro"])).toEqual([defaultKey, pro]);
});
