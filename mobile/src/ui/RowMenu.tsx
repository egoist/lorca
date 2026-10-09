// A row whose tap opens its menu: SwiftUI's on iOS, labelled for VoiceOver with the row's words, and
// Material's dropdown on Android. Destructive actions sit in a group of their own.

import { MenuView } from "@expo/ui/community/menu";
import { Button as SwiftButton, Host as SwiftHost, Menu as SwiftMenu, RNHostView, Section as SwiftSection } from "@expo/ui/swift-ui";
import { accessibilityHint, accessibilityLabel, disabled } from "@expo/ui/swift-ui/modifiers";
import { useState, type ReactNode } from "react";
import { Platform, View } from "react-native";

export type RowAction<Id extends string> = {
  id: Id;
  title: string;
  /// The SF Symbol iOS shows beside it.
  symbol?: string;
  disabled?: boolean;
  destructive?: boolean;
};

export function RowMenu<Id extends string>({ actions, onChoose, label, hint, children }: { actions: RowAction<Id>[]; onChoose: (action: Id) => void; label: string; hint?: string; children: ReactNode }) {
  // The menu's host takes its size from what it is given; Android delivers no touch outside it.
  const [width, setWidth] = useState(0);
  const groups = [actions.filter((action) => !action.destructive), actions.filter((action) => action.destructive)].filter((group) => group.length > 0);
  if (Platform.OS === "ios") {
    return (
      <View onLayout={(e) => setWidth(e.nativeEvent.layout.width)}>
        <SwiftHost matchContents style={{ width }} ignoreSafeArea="all">
          <SwiftMenu
            label={
              <RNHostView matchContents>
                <View style={{ width }}>{children}</View>
              </RNHostView>
            }
            modifiers={[accessibilityLabel(label), ...(hint ? [accessibilityHint(hint)] : [])]}
          >
            {groups.map((group) => (
              <SwiftSection key={group[0].id}>
                {group.map((action) => (
                  <SwiftButton
                    key={action.id}
                    label={action.title}
                    systemImage={action.symbol as never}
                    role={action.destructive ? "destructive" : undefined}
                    modifiers={action.disabled ? [disabled(true)] : undefined}
                    onPress={() => onChoose(action.id)}
                  />
                ))}
              </SwiftSection>
            ))}
          </SwiftMenu>
        </SwiftHost>
      </View>
    );
  }
  return (
    <View onLayout={(e) => setWidth(e.nativeEvent.layout.width)}>
      <MenuView
        style={{ width }}
        actions={groups.map((group) => ({
          id: group[0].id,
          title: "",
          displayInline: true,
          subactions: group.map((action) => ({ id: action.id, title: action.title, attributes: { disabled: action.disabled, destructive: action.destructive } })),
        }))}
        onPressAction={({ nativeEvent }) => onChoose(nativeEvent.event as Id)}
      >
        <View style={{ width }} accessibilityLabel={label} accessibilityHint={hint}>
          {children}
        </View>
      </MenuView>
    </View>
  );
}
