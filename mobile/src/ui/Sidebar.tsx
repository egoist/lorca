// The chat list as the sidebar of a wide window. It is a native stack of its own, one screen
// deep, so the list keeps the native bar, search field and toolbar buttons it has on a phone.

import { DarkTheme, DefaultTheme } from "expo-router";
import { createNativeStackNavigator } from "expo-router/build/fork/native-stack/createNativeStackNavigator";
import { NavigationContainer, NavigationIndependentTree, useTheme } from "expo-router/react-navigation";
import { Platform } from "react-native";
import { t, useLanguage } from "../i18n";
import { ChatsScreen } from "./ChatsScreen";
import { useStackScreenOptions } from "./navigation";

const SidebarStack = createNativeStackNavigator();

function SidebarChats() {
  return <ChatsScreen sidebar />;
}

export function Sidebar() {
  useLanguage();
  const theme = useTheme();
  const screenOptions = useStackScreenOptions();
  return (
    <NavigationIndependentTree>
      <NavigationContainer theme={{ ...(theme.dark ? DarkTheme : DefaultTheme), ...theme } as any}>
        <SidebarStack.Navigator screenOptions={screenOptions as any}>
          <SidebarStack.Screen
            name="chats"
            component={SidebarChats}
            options={{ title: t("Chats"), headerTitle: "", headerLargeTitle: false, headerShadowVisible: false, headerTransparent: Platform.OS === "ios" }}
          />
        </SidebarStack.Navigator>
      </NavigationContainer>
    </NavigationIndependentTree>
  );
}
