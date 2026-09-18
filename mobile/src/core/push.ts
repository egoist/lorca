// Pushes for finished replies. The phone gives the relay its APNs or FCM device token; a
// Runner seals who replied and the first words, and the phone opens that before the alert
// shows (the notification service extension on iOS, PushService on Android). Here: the
// permission, the token, what shows while the app is open, and the tap that opens the chat.

import * as Application from "expo-application";
import * as Notifications from "expo-notifications";
import { router } from "expo-router";
import { Platform } from "react-native";
import * as core from "../../modules/lorca-core";
import { useStore } from "./store";

/// The chat a notification is about. iOS carries it as the data the extension set; Android's
/// notification opens the chat by link and never comes through here.
function chatOf(notification: Notifications.Notification): string | undefined {
  const id = notification.request.content.data?.chat_id;
  return typeof id === "string" ? id : undefined;
}

let installed = false;

/// Once, at launch: a reply in the chat on screen makes no banner, and a tapped notification
/// opens its chat (also the one that launched the app).
export function installPushHandlers() {
  if (installed) return;
  installed = true;
  Notifications.setNotificationHandler({
    handleNotification: async (notification) => {
      const show = chatOf(notification) !== useStore.getState().openChatId;
      return { shouldShowBanner: show, shouldShowList: show, shouldPlaySound: show, shouldSetBadge: false };
    },
  });
  const open = (response: Notifications.NotificationResponse | null) => {
    const chatId = response && chatOf(response.notification);
    if (chatId && useStore.getState().paired) router.navigate({ pathname: "/chat/[id]", params: { id: chatId } });
  };
  Notifications.addNotificationResponseReceivedListener(open);
  open(Notifications.getLastNotificationResponse());
}

/// Asks for permission the first time and registers this phone's token with the relay. Safe
/// to call on every launch and foreground; a phone without push (a simulator without APNs,
/// an Android build without Firebase) just has none.
export async function registerForPushes() {
  try {
    let { status } = await Notifications.getPermissionsAsync();
    if (status === "undetermined") ({ status } = await Notifications.requestPermissionsAsync());
    if (status !== "granted") return;
    const token = await Notifications.getDevicePushTokenAsync();
    if (typeof token.data !== "string") return;
    // Only a store or TestFlight build says "production"; a development build and the simulator take sandbox pushes.
    const environment = Platform.OS === "ios" ? ((await Application.getIosPushNotificationServiceEnvironmentAsync()) === "production" ? "production" : "sandbox") : undefined;
    await core.request("push.register", { platform: Platform.OS === "ios" ? "apns" : "fcm", token: token.data, environment });
  } catch (error) {
    console.warn("registering for pushes", error instanceof Error ? error.message : error);
  }
}

/// The chat is on screen: what was posted for it has been seen.
export async function clearPushes(chatId: string) {
  try {
    const presented = await Notifications.getPresentedNotificationsAsync();
    await Promise.all(presented.filter((n) => chatOf(n) === chatId).map((n) => Notifications.dismissNotificationAsync(n.request.identifier)));
  } catch {}
}
