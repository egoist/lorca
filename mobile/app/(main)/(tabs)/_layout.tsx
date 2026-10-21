import { Stack, usePathname, useRouter } from "expo-router";
import { NativeTabs } from "expo-router/unstable-native-tabs";
import { useEffect, useState } from "react";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../src/i18n";
import { ChatsBar } from "../../../src/ui/ChatsScreen";
import { useWide } from "../../../src/ui/layout";
import { usePalette } from "../../../src/ui/theme";

/// A phone's home: Bots (the chat list), Devices, and Search, in the system's tab bar. Beside the sidebar the bar is hidden: the sidebar holds
/// the list and its search, and Settings the Devices.
///
/// On iOS the Bots tab's bar is the bar of the `(main)` stack around the tabs, not one of its own:
/// a chat slides into that stack, and one navigation bar fades the list's buttons into the chat's
/// back button and title while the bar's blur stays put. A bar of the tab's own would slide out
/// with the list as the chat's slid in. The other tabs keep their stacks' bars.
export default function TabsLayout() {
  useLanguage();
  const p = usePalette();
  const wide = useWide();
  const router = useRouter();
  const pathname = usePathname();
  const [tab, setTab] = useState("(chats)");
  const sharedBar = Platform.OS === "ios" && !wide && tab === "(chats)";

  // A window that widens on another tab goes back to the empty pane beside the sidebar.
  useEffect(() => {
    if (wide && (pathname.startsWith("/devices") || pathname === "/search")) router.navigate("/");
    // Only when the window widens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wide]);

  return (
    <>
      <Stack.Screen options={{ headerShown: sharedBar, headerTitle: "", headerTransparent: true, headerShadowVisible: false }} />
      {sharedBar ? <ChatsBar /> : null}
      <NativeTabs hidden={wide} tintColor={p.tint} screenListeners={({ route }) => ({ focus: () => setTab(route.name) })}>
        <NativeTabs.Trigger name="(chats)">
          <NativeTabs.Trigger.Icon sf={{ default: "bubble.left.and.bubble.right", selected: "bubble.left.and.bubble.right.fill" }} md="forum" />
          <NativeTabs.Trigger.Label>{t("Bots")}</NativeTabs.Trigger.Label>
        </NativeTabs.Trigger>
        <NativeTabs.Trigger name="devices">
          <NativeTabs.Trigger.Icon sf="laptopcomputer.and.iphone" md="devices" />
          <NativeTabs.Trigger.Label>{t("Devices")}</NativeTabs.Trigger.Label>
        </NativeTabs.Trigger>
        <NativeTabs.Trigger name="search" role="search">
          <NativeTabs.Trigger.Icon sf="magnifyingglass" md="search" />
          <NativeTabs.Trigger.Label>{t("Search")}</NativeTabs.Trigger.Label>
        </NativeTabs.Trigger>
      </NativeTabs>
    </>
  );
}
