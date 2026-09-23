// A container whose contents shimmer, as the Mac's working row does: dimmed, with a bright band
// sweeping across them from the leading edge to the trailing one.
import { requireNativeViewManager } from "expo-modules-core";
import type { ViewProps } from "react-native";

export const ShimmerView: React.ComponentType<ViewProps> = requireNativeViewManager<ViewProps>("ShimmerView");
