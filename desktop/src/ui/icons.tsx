// Symbols. Bots, plugins, and the macOS app name their looks by SF Symbol ("sparkles",
// "chevron.left.forwardslash.chevron.right"); on Windows and Linux each name draws the Lucide icon
// that says the same thing.

import {
  Activity,
  AppWindow,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  ArrowUpRight,
  AtSign,
  AudioWaveform,
  Binoculars,
  Book,
  BookOpen,
  BookText,
  Box,
  Brain,
  Calendar,
  ChartColumnBig,
  ChartLine,
  ChartNoAxesColumn,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronsUpDown,
  CircleAlert,
  CircleCheck,
  CircleDollarSign,
  CircleMinus,
  CirclePause,
  CirclePlus,
  ClipboardList,
  Clock,
  Cloud,
  CodeXml,
  Computer,
  Contact,
  Copy,
  CreditCard,
  Crown,
  Database,
  Ellipsis,
  Eye,
  EyeOff,
  File,
  FileText,
  FlaskConical,
  Flame,
  Folder,
  GitBranch,
  Globe,
  Hammer,
  Hand,
  IdCard,
  Inbox,
  Info,
  Key,
  KeyRound,
  Landmark,
  Laptop,
  LayoutGrid,
  Leaf,
  Link,
  ListChecks,
  ListFilter,
  Mail,
  MessageCircle,
  MessagesSquare,
  Mic,
  Monitor,
  NotebookText,
  Paintbrush,
  Palette,
  PanelLeft,
  PanelRight,
  Paperclip,
  PencilLine,
  PhoneOutgoing,
  Pin,
  Plus,
  Puzzle,
  QrCode,
  RefreshCw,
  Search,
  Send,
  Server,
  Settings,
  Shield,
  ShieldCheck,
  Signature,
  SlidersHorizontal,
  Smartphone,
  Smile,
  Sparkles,
  Square,
  SquareDashed,
  SquarePen,
  SquareTerminal,
  Sun,
  Table,
  Tablet,
  Terminal,
  TextSearch,
  Ticket,
  TimerReset,
  TrainFront,
  Trash2,
  TrendingUp,
  TriangleAlert,
  User,
  UserCheck,
  UserPlus,
  Users,
  WandSparkles,
  Wrench,
  X,
  Zap,
  ArrowDownToLine,
  Circle,
  CircleQuestionMark,
  CircleStop,
  Maximize2,
  MessageSquarePlus,
  Network,
  Pencil,
  Menu,
  Reply,
  CircleX,
  type IconNode,
} from "lucide";
import { createMemo } from "solid-js";
import type { JSX } from "@solidjs/web";

const symbols: Record<string, IconNode> = {
  // The app's own controls.
  plus: Plus,
  "plus.circle.fill": CirclePlus,
  "plus.message": MessageSquarePlus,
  "line.3.horizontal": Menu,
  circle: Circle,
  pencil: Pencil,
  network: Network,
  "questionmark.circle": CircleQuestionMark,
  "stop.circle": CircleStop,
  "arrow.down.to.line": ArrowDownToLine,
  "arrow.up.left.and.arrow.down.right": Maximize2,
  "sidebar.leading": PanelLeft,
  "sidebar.trailing": PanelRight,
  terminal: Terminal,
  "terminal.fill": SquareTerminal,
  gearshape: Settings,
  "gearshape.fill": Settings,
  "circle.grid.2x2": LayoutGrid,
  "pin.fill": Pin,
  pin: Pin,
  "arrow.down": ArrowDown,
  "arrow.up": ArrowUp,
  "arrow.right": ArrowRight,
  "arrow.up.right": ArrowUpRight,
  "mic.fill": Mic,
  "stop.fill": Square,
  xmark: X,
  "xmark.circle.fill": CircleX,
  "arrowshape.turn.up.left": Reply,
  "arrowshape.turn.up.left.fill": Reply,
  "doc.fill": File,
  "chevron.left": ChevronLeft,
  "chevron.right": ChevronRight,
  "chevron.down": ChevronDown,
  "chevron.up.chevron.down": ChevronsUpDown,
  "clock.badge.questionmark": TimerReset,
  "hand.raised": Hand,
  "person.crop.circle.badge.checkmark": UserCheck,
  "puzzlepiece.extension": Puzzle,
  "minus.circle": CircleMinus,
  "bubble.left": MessageCircle,
  checkmark: Check,
  "checkmark.circle": CircleCheck,
  "checkmark.circle.fill": CircleCheck,
  "exclamationmark.circle.fill": CircleAlert,
  clock: Clock,
  "pause.circle": CirclePause,
  "arrow.triangle.2.circlepath": RefreshCw,
  "square.and.pencil": SquarePen,
  trash: Trash2,
  key: Key,
  "key.fill": KeyRound,
  "person.badge.key.fill": IdCard,
  "checkmark.shield": ShieldCheck,
  "slider.horizontal.3": SlidersHorizontal,
  "person.2": Users,
  "person.2.fill": Users,
  "person.fill": User,
  "person.badge.plus": UserPlus,
  "person.text.rectangle": Contact,
  magnifyingglass: Search,
  eye: Eye,
  "eye.fill": Eye,
  "eye.slash": EyeOff,
  "doc.on.doc": Copy,
  qrcode: QrCode,
  "pencil.line": PencilLine,
  "text.magnifyingglass": TextSearch,
  folder: Folder,
  "doc.text": FileText,
  "doc.text.fill": FileText,
  "wrench.and.screwdriver.fill": Wrench,
  "arrow.triangle.turn.up.right.diamond.fill": Send,
  ellipsis: Ellipsis,
  link: Link,
  at: AtSign,
  paperclip: Paperclip,
  "info.circle": Info,
  "exclamationmark.triangle": TriangleAlert,
  "exclamationmark.triangle.fill": TriangleAlert,
  // Devices.
  laptopcomputer: Laptop,
  desktopcomputer: Monitor,
  display: Monitor,
  macstudio: Computer,
  macmini: Computer,
  "server.rack": Server,
  pc: Computer,
  iphone: Smartphone,
  ipad: Tablet,
  smartphone: Smartphone,
  // Bots' looks.
  sparkles: Sparkles,
  "wand.and.stars": WandSparkles,
  "hammer.fill": Hammer,
  "book.fill": Book,
  "paintbrush.fill": Paintbrush,
  "chart.bar.fill": ChartColumnBig,
  globe: Globe,
  "brain.head.profile": Brain,
  "envelope.fill": Mail,
  calendar: Calendar,
  "flask.fill": FlaskConical,
  "bolt.fill": Zap,
  "leaf.fill": Leaf,
  "shield.fill": Shield,
  "binoculars.fill": Binoculars,
  binoculars: Binoculars,
  "chevron.left.forwardslash.chevron.right": CodeXml,
  "pencil.and.scribble": Signature,
  "bolt.horizontal.fill": Activity,
  "flame.fill": Flame,
  "list.bullet.clipboard.fill": ClipboardList,
  "sun.max.fill": Sun,
  checklist: ListChecks,
  "tray.full.fill": Inbox,
  "chart.line.uptrend.xyaxis": TrendingUp,
  "crown.fill": Crown,
  // Plugins.
  "arrow.triangle.branch": GitBranch,
  "bolt.horizontal": Zap,
  "bolt.horizontal.circle": Zap,
  "book.closed": BookText,
  "book.pages": BookOpen,
  "building.columns": Landmark,
  "bubble.left.and.bubble.right": MessagesSquare,
  "chart.bar.xaxis": ChartNoAxesColumn,
  "chart.xyaxis.line": ChartLine,
  cloud: Cloud,
  creditcard: CreditCard,
  cube: Box,
  cylinder: Database,
  "cylinder.split.1x2": Database,
  "doc.richtext": FileText,
  "dollarsign.circle": CircleDollarSign,
  envelope: Mail,
  "face.smiling": Smile,
  "line.3.horizontal.decrease.circle": ListFilter,
  macwindow: AppWindow,
  "note.text": NotebookText,
  paintpalette: Palette,
  "person.crop.rectangle.stack": Contact,
  "phone.arrow.up.right": PhoneOutgoing,
  "square.on.square.dashed": SquareDashed,
  tablecells: Table,
  ticket: Ticket,
  "train.side.front.car": TrainFront,
  waveform: AudioWaveform,
};

/** Symbols SF draws solid, which the stroke icons fill to match. */
const filled = new Set(["stop.fill", "pin.fill"]);

function escape(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;");
}

function markup(node: IconNode): string {
  return node
    .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([key, value]) => `${key}="${escape(String(value))}"`).join(" ")}/>`)
    .join("");
}

const cache = new Map<string, string>();

function symbolMarkup(name: string): string {
  let found = cache.get(name);
  if (found === undefined) {
    found = markup(symbols[name] ?? Sparkles);
    cache.set(name, found);
  }
  return found;
}

export interface IconProps {
  /** The SF Symbol name. */
  name: string;
  size?: number;
  strokeWidth?: number;
  class?: string;
  style?: JSX.CSSProperties;
  /** What the symbol means, for a symbol that is the only label of a control. */
  label?: string;
}

export function Icon(props: IconProps) {
  const inner = createMemo(() => symbolMarkup(props.name));
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={props.size ?? 16}
      height={props.size ?? 16}
      viewBox="0 0 24 24"
      fill={filled.has(props.name) ? "currentColor" : "none"}
      stroke="currentColor"
      stroke-width={props.strokeWidth ?? 2}
      stroke-linecap="round"
      stroke-linejoin="round"
      class={["icon", props.class]}
      style={props.style}
      role={props.label ? "img" : undefined}
      aria-label={props.label}
      aria-hidden={props.label ? undefined : "true"}
      innerHTML={inner()}
    />
  );
}
