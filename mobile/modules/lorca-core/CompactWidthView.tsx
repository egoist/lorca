// A container that lays its contents out for a compact width on iOS, as a split view's sidebar
// column does. Android has no size classes to override; there it is a plain view.
import { requireNativeViewManager } from "expo-modules-core";
import { Platform, View, type ViewProps } from "react-native";

export const CompactWidthView: React.ComponentType<ViewProps> = Platform.OS === "ios" ? requireNativeViewManager<ViewProps>("CompactWidthView") : View;
