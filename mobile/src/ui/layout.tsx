// The window's shape. A wide window (a tablet, or a tablet window with room to spare) shows the
// chat list as a sidebar beside the open chat; a narrow one stacks the chat over the list.

import { createContext, use } from "react";
import { useWindowDimensions } from "react-native";

/// An iPad mini in portrait (744 pt) is wide; half of an 11-inch iPad's screen is not.
const WIDE_MIN = 700;

export function useWide(): boolean {
  return useWindowDimensions().width >= WIDE_MIN;
}

export function useSidebarWidth(): number {
  return useWindowDimensions().width >= 1100 ? 375 : 320;
}

const PaneWidthContext = createContext<number | null>(null);

/// The width of the pane its children are laid out in, where that is not the window's.
export const PaneWidth = PaneWidthContext.Provider;

export function usePaneWidth(): number {
  const pane = use(PaneWidthContext);
  const { width } = useWindowDimensions();
  return pane ?? width;
}
