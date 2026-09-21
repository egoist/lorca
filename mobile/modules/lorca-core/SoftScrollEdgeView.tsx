// A container that gives the scroll view inside it a soft top edge on iOS, where an iPad would
// pick the hard one and rule a line under a transparent bar. A plain view on Android.
import { requireNativeViewManager } from "expo-modules-core";
import { Platform, View, type ViewProps } from "react-native";

export const SoftScrollEdgeView: React.ComponentType<ViewProps> = Platform.OS === "ios" ? requireNativeViewManager<ViewProps>("SoftScrollEdgeView") : View;
