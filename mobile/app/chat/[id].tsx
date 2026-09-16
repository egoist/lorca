import { FlashList, type FlashListRef } from "@shopify/flash-list";
import * as Clipboard from "expo-clipboard";
import * as Haptics from "expo-haptics";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActionSheetIOS, Alert, Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { KeyboardAvoidingView, KeyboardEvents } from "react-native-keyboard-controller";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { chatTitle, engine } from "../../src/core/engine";
import type { Bot, Message } from "../../src/core/model";
import { markRead, useBotMap, useChat, useIsWorking, useStore, useWorkingBots } from "../../src/core/store";
import { AvatarCluster } from "../../src/ui/Avatar";
import { Composer } from "../../src/ui/Composer";
import { usePalette } from "../../src/ui/theme";
import { buildRows, DayRow, MarkerRow, MessageRow, NoticeRow, StatusRow, WorkingRow, type Row } from "../../src/ui/transcript";

export default function ChatScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const insets = useSafeAreaInsets();
  const chat = useChat(id);
  const bots = useBotMap();
  const workingBotIds = useWorkingBots(id);
  const isWorking = useIsWorking(id);
  const status = useStore((s) => s.statuses[id] ?? null);
  const listRef = useRef<FlashListRef<Row>>(null);
  /// The composer floats over the transcript; the list keeps that much room at its end.
  const [composerHeight, setComposerHeight] = useState(70);

  useEffect(() => {
    useStore.setState({ openChatId: id });
    markRead(id);
    return () => {
      if (useStore.getState().openChatId === id) useStore.setState({ openChatId: null });
    };
  }, [id]);

  useEffect(() => {
    if (chat && chat.unread_count > 0) markRead(id);
  }, [chat, id]);

  useEffect(() => {
    const sub = KeyboardEvents.addListener("keyboardWillShow", () => {
      listRef.current?.scrollToEnd({ animated: true });
    });
    return () => sub.remove();
  }, []);

  const rows = useMemo(() => (chat ? buildRows(chat, bots, workingBotIds, isWorking, status) : []), [chat, bots, workingBotIds, isWorking, status]);
  const members = useMemo(() => (chat?.bot_ids ?? []).map((b) => bots.get(b)).filter((b): b is Bot => !!b), [chat, bots]);
  const isGroup = chat?.kind === "group";
  const title = chat ? chatTitle(chat) : "Chat";
  // After the Mac app: "Message Chef", or the group's title with a hint that @ addresses one bot.
  const placeholder = !isGroup && members[0] ? `Message ${members[0].name}` : members.length > 1 ? `Message ${title} — @ to address one bot` : `Message ${title}`;

  const onLongPress = useCallback((message: Message) => {
    const text = message.body.kind === "text" ? message.body.text : "";
    const copy = () => {
      void Clipboard.setStringAsync(text);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    };
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions({ options: ["Copy", "Cancel"], cancelButtonIndex: 1 }, (index) => {
        if (index === 0) copy();
      });
    } else {
      Alert.alert("Message", undefined, [
        { text: "Copy", onPress: copy },
        { text: "Cancel", style: "cancel" },
      ]);
    }
  }, []);

  if (!chat) {
    return (
      <View style={styles.missing}>
        <Stack.Screen options={{ title: "Chat" }} />
        <Text style={{ color: p.secondaryLabel }}>This chat is no longer on the roster.</Text>
      </View>
    );
  }

  return (
    <KeyboardAvoidingView behavior="padding" style={{ flex: 1, backgroundColor: p.background }} keyboardVerticalOffset={0}>
      <Stack.Screen options={{ title }} />
      <Stack.Title asChild>
        <Pressable onPress={() => router.push(`/chat-info/${id}`)} style={styles.titleView} accessibilityLabel={`${title}, info`}>
          <AvatarCluster bots={members} size={30} working={isWorking} />
          <Text style={[styles.titleText, { color: p.label }]} numberOfLines={1}>
            {title}
          </Text>
        </Pressable>
      </Stack.Title>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button icon="info.circle" accessibilityLabel="Chat info" onPress={() => router.push(`/chat-info/${id}`)} />
      </Stack.Toolbar>
      <FlashList
        ref={listRef}
        data={rows}
        keyExtractor={(row) => row.key}
        getItemType={(row) => row.type}
        contentInsetAdjustmentBehavior="automatic"
        keyboardDismissMode="interactive"
        maintainVisibleContentPosition={{ startRenderingFromBottom: true, autoscrollToBottomThreshold: 0.25, animateAutoScrollToBottom: true }}
        contentContainerStyle={{ paddingBottom: composerHeight + 6, paddingTop: Platform.OS === "ios" ? 0 : 8 }}
        scrollIndicatorInsets={{ bottom: composerHeight }}
        renderItem={({ item }) => {
          switch (item.type) {
            case "day":
              return <DayRow at={item.at} />;
            case "message":
              return <MessageRow row={item} bots={bots} isGroup={isGroup} onLongPress={onLongPress} />;
            case "marker":
              return <MarkerRow row={item} />;
            case "notice":
              return <NoticeRow row={item} />;
            case "working":
              return <WorkingRow bots={item.bots} isGroup={isGroup} />;
            case "status":
              return <StatusRow text={item.text} />;
          }
        }}
      />
      <View style={[styles.composer, { paddingBottom: Math.max(insets.bottom, 8) }]} onLayout={(e) => setComposerHeight(e.nativeEvent.layout.height)} pointerEvents="box-none">
        <Composer
          members={members}
          isGroup={isGroup}
          placeholder={placeholder}
          onSend={(text, files) => {
            engine.sendMessage(id, text, files).catch((error) => {
              Alert.alert("Could not send", error instanceof Error ? error.message : String(error));
            });
          }}
        />
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  missing: { flex: 1, alignItems: "center", justifyContent: "center" },
  composer: { position: "absolute", left: 0, right: 0, bottom: 0 },
  titleView: { flexDirection: "row", alignItems: "center", gap: 8, maxWidth: 240 },
  titleText: { fontSize: 17, fontWeight: "600", flexShrink: 1 },
});
