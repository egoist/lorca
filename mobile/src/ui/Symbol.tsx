// SF Symbols, the same names the Mac app and bot profiles use, with Material Symbols standing
// in on Android.

import { SymbolView, type SymbolViewProps, type SymbolWeight } from "expo-symbols";
import type { ColorValue, StyleProp, ViewStyle } from "react-native";

const ANDROID: Record<string, string> = {
  sparkles: "auto_awesome",
  "person.fill": "person",
  "person.2.fill": "group",
  "person.badge.plus": "person_add",
  "gearshape.fill": "settings",
  gearshape: "settings",
  "square.and.pencil": "edit_square",
  "info.circle": "info",
  "arrow.left": "arrow_back",
  "arrow.up": "arrow_upward",
  "stop.fill": "stop",
  "pin.fill": "push_pin",
  pin: "push_pin",
  "pin.slash": "push_pin",
  trash: "delete",
  "checkmark.circle.fill": "check_circle",
  checkmark: "check",
  "circle.fill": "circle",
  "qrcode.viewfinder": "qr_code_scanner",
  "doc.on.clipboard": "content_paste",
  "exclamationmark.triangle.fill": "warning",
  "info.circle.fill": "info",
  laptopcomputer: "laptop_mac",
  desktopcomputer: "desktop_mac",
  macstudio: "desktop_mac",
  macmini: "desktop_mac",
  "server.rack": "dns",
  pc: "computer",
  iphone: "smartphone",
  ipad: "tablet",
  smartphone: "smartphone",
  "questionmark.circle": "help",
  "wand.and.stars": "auto_fix_high",
  "hammer.fill": "handyman",
  "book.fill": "menu_book",
  "book.closed": "menu_book",
  "paintbrush.fill": "brush",
  "chart.bar.fill": "bar_chart",
  "terminal.fill": "terminal",
  terminal: "terminal",
  "globe": "public",
  "brain.head.profile": "psychology",
  "magnifyingglass": "search",
  "envelope.fill": "mail",
  "calendar": "calendar_month",
  "flask.fill": "science",
  "bolt.fill": "bolt",
  "leaf.fill": "eco",
  "music.note": "music_note",
  "camera.fill": "photo_camera",
  "shield.fill": "shield",
  "hand.raised": "pan_tool",
  "cart.fill": "shopping_cart",
  "arrow.triangle.turn.up.right.diamond.fill": "alt_route",
  "chevron.right": "chevron_right",
  // A value menu: Material's dropdown arrow.
  "chevron.up.chevron.down": "arrow_drop_down",
  "arrow.down": "arrow_downward",
  "plus": "add",
  // Durable tasks' states and evidence.
  "circle": "radio_button_unchecked",
  "arrow.triangle.2.circlepath": "sync",
  "exclamationmark.circle.fill": "error",
  "checkmark.circle": "check_circle",
  "xmark.circle": "cancel",
  "bubble.left": "chat_bubble",
  play: "play_arrow",
  "arrow.counterclockwise": "replay",
  "xmark": "close",
  // Android's overflow menu is the vertical one.
  "ellipsis": "more_vert",
  "crown.fill": "workspace_premium",
  "key.fill": "key",
  key: "key",
  lock: "lock",
  "person.badge.key.fill": "badge",
  "antenna.radiowaves.left.and.right": "cell_tower",
  "link": "link",
  "at": "alternate_email",
  "mic.fill": "mic",
  "waveform": "graphic_eq",
  "paperclip": "attach_file",
  flag: "flag",
  "checkmark.seal": "verified",
  "photo.on.rectangle": "photo_library",
  photo: "image",
  "doc.fill": "description",
  doc: "draft",
  "doc.text": "description",
  "doc.richtext": "picture_as_pdf",
  film: "movie",
  eye: "visibility",
  safari: "open_in_browser",
  "square.and.arrow.up": "share",
  "square.and.arrow.down": "download",
  "doc.on.doc": "content_copy",
  "arrow.clockwise": "refresh",
  "folder.fill": "folder",
  "xmark.circle.fill": "cancel",
  "arrowshape.turn.up.left.fill": "reply",
  "binoculars.fill": "travel_explore",
  "text.bubble": "chat",
  "person.crop.circle.badge.checkmark": "how_to_reg",
  "bubble.left.and.bubble.right": "forum",
  "chevron.left.forwardslash.chevron.right": "code",
  "pencil.and.scribble": "draw",
  "bolt.horizontal.fill": "electric_bolt",
  "flame.fill": "local_fire_department",
  "puzzlepiece.extension": "extension",
  "doc.text.magnifyingglass": "pageview",
  // Workflow feedback.
  "hand.thumbsup": "thumb_up",
  pencil: "edit",
  "pencil.line": "edit_note",
  "arrow.uturn.backward": "undo",
  "eye.slash": "visibility_off",
  "exclamationmark.triangle": "warning",
  // Workflows.
  envelope: "mail",
  "arrow.triangle.branch": "call_split",
  "point.3.connected.trianglepath.dotted": "account_tree",
};

export interface SymbolProps {
  name: string;
  size?: number;
  color?: ColorValue;
  weight?: SymbolWeight;
  style?: StyleProp<ViewStyle>;
  type?: SymbolViewProps["type"];
}

export function Symbol({ name, size = 17, color, weight = "medium", style, type }: SymbolProps) {
  return (
    <SymbolView
      name={{ ios: name as any, android: (ANDROID[name] ?? "auto_awesome") as any }}
      size={size}
      tintColor={color}
      weight={weight}
      type={type}
      resizeMode="scaleAspectFit"
      style={[{ width: size, height: size }, style]}
    />
  );
}

/// Symbols offered when creating a bot, a subset the Mac app also knows.
export const BOT_SYMBOLS = [
  "sparkles",
  "wand.and.stars",
  "hammer.fill",
  "book.fill",
  "paintbrush.fill",
  "chart.bar.fill",
  "terminal.fill",
  "globe",
  "brain.head.profile",
  "magnifyingglass",
  "envelope.fill",
  "calendar",
  "flask.fill",
  "bolt.fill",
  "leaf.fill",
  "shield.fill",
  "binoculars.fill",
  "chevron.left.forwardslash.chevron.right",
  "pencil.and.scribble",
  "bolt.horizontal.fill",
  "flame.fill",
];
