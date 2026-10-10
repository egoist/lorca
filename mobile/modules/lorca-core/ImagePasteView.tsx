// A container that takes images pasted into the text input inside it, from the edit menu, a
// hardware keyboard, or (on Android) the soft keyboard's image insertion. Each image arrives as
// a file in the app's cache.
import { requireNativeViewManager } from "expo-modules-core";
import type { NativeSyntheticEvent, ViewProps } from "react-native";

export interface PastedImage {
  /** A `file://` URI. */
  uri: string;
  mime: string;
  size: number;
  width?: number;
  height?: number;
}

export interface ImagePasteViewProps extends ViewProps {
  onPasteImages?: (event: NativeSyntheticEvent<{ files: PastedImage[] }>) => void;
}

export const ImagePasteView: React.ComponentType<ImagePasteViewProps> = requireNativeViewManager<ImagePasteViewProps>("ImagePasteView");
