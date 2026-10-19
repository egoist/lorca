import { ScrollView } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { useDeviceRows } from "../../../../src/ui/devices";
import { Section } from "../../../../src/ui/forms";

/// The Devices tab: every Device on the account, each sliding in its screen.
export default function DevicesScreen() {
  useLanguage();
  const rows = useDeviceRows();
  return (
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
      <Section footer={t("Desktop Devices are Runners: they run bots with your account's provider credentials. Phones and tablets read and write chats.")}>{rows}</Section>
    </ScrollView>
  );
}
