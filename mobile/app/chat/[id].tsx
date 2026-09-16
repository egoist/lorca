import { FlashList, type FlashListRef } from "@shopify/flash-list";
import * as Clipboard from "expo-clipboard";
import * as Haptics from "expo-haptics";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useHeaderHeight } from "expo-router/react-navigation";
import {
  forwardRef,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ActionSheetIOS,
  Alert,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Platform,
  Pressable,
  type ScrollViewProps,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  KeyboardChatScrollView,
  KeyboardStickyView,
} from "react-native-keyboard-controller";
import { useSharedValue } from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { chatTitle, engine } from "../../src/core/engine";
import type { Bot, Message } from "../../src/core/model";
import {
  markRead,
  useBotMap,
  useChat,
  useIsWorking,
  useStore,
  useWorkingBots,
} from "../../src/core/store";
import { AvatarCluster } from "../../src/ui/Avatar";
import { Composer } from "../../src/ui/Composer";
import { usePalette } from "../../src/ui/theme";
import {
  buildRows,
  DayRow,
  MarkerRow,
  MessageRow,
  NoticeRow,
  StatusRow,
  WorkingRow,
  type Row,
} from "../../src/ui/transcript";

/// Breathing room between the last message and the composer, as on the Mac.
const COMPOSER_GAP = 14;

export default function ChatScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const insets = useSafeAreaInsets();
  const headerHeight = useHeaderHeight();
  const chat = useChat(id);
  const bots = useBotMap();
  const workingBotIds = useWorkingBots(id);
  const isWorking = useIsWorking(id);
  const status = useStore((s) => s.statuses[id] ?? null);
  const listRef = useRef<FlashListRef<Row>>(null);
  // The composer floats over the transcript and rides the keyboard (KeyboardStickyView). The
  // list is never resized: the chat scroll view keeps a bottom inset for the composer and adds
  // the keyboard's height to it frame by frame, lifting the last messages with the keys.
  // `blankSpace` is the inset with the keyboard closed (the composer with its home-indicator
  // padding); `extraContentPadding` is the composer's height above the keys once it is open.
  // Composer bar (46) + its wrap padding (8) + home-indicator padding + the gap, as in onLayout.
  const composerGuess = 54 + COMPOSER_GAP + Math.max(insets.bottom, 8);
  const composerBlank = useSharedValue(composerGuess);
  const composerExtra = useSharedValue(composerGuess - insets.bottom);
  // Opening a chat: FlashList lays the last rows out from the bottom, but their measured heights,
  // the composer's inset and the header inset all land over the next few frames, and the composer
  // inset reaches the native scroll view a frame after JS sets it. So the end is computed here
  // from the content height, the viewport and the composer inset rather than read from the
  // scroll view, and the list stays invisible until a scroll event confirms it sits there.
  // Until the user drags (or the list has been quiet for a moment after its first load), every
  // change re-pins it; `settled` also keeps FlashList's own catch-up scrolls instant meanwhile.
  const settling = useRef(true);
  const pinQueued = useRef(false);
  const pinIssued = useRef(false);
  const loaded = useRef(false);
  // True while the last thing that happened was a scroll event landing on the computed end.
  const confirmed = useRef(false);
  const lastOffset = useRef<number | null>(null);
  const [settled, setSettled] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const settleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const revealTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const layoutHeight = useRef(0);
  const contentHeight = useRef(0);
  const topInset = Platform.OS === "ios" ? headerHeight : 0;
  const topInsetRef = useRef(topInset);
  topInsetRef.current = topInset;
  const endOffset = useCallback(
    () =>
      Math.max(
        -topInsetRef.current,
        contentHeight.current + composerBlank.value - layoutHeight.current,
      ),
    [composerBlank],
  );
  // Shows the list once it has been quiet for a few frames after a confirmed pin: FlashList
  // can re-lay the content out under an offset that was right an instant earlier.
  const scheduleReveal = useCallback(() => {
    if (revealTimer.current) clearTimeout(revealTimer.current);
    revealTimer.current = setTimeout(() => {
      revealTimer.current = null;
      if (confirmed.current && loaded.current) setRevealed(true);
    }, 50);
  }, []);
  const confirm = useCallback(() => {
    confirmed.current = true;
    scheduleReveal();
  }, [scheduleReveal]);
  const unconfirm = useCallback(() => {
    confirmed.current = false;
    if (revealTimer.current) clearTimeout(revealTimer.current);
    revealTimer.current = null;
  }, []);
  const pinToBottom = useCallback(() => {
    if (!settling.current || pinQueued.current) return;
    pinQueued.current = true;
    requestAnimationFrame(() => {
      pinQueued.current = false;
      if (!settling.current) return;
      if (Platform.OS === "ios") {
        if (layoutHeight.current === 0 || contentHeight.current === 0) return;
        pinIssued.current = true;
        const offset = endOffset();
        // Already there: the native side skips the no-op scroll, so no event will confirm it.
        const already =
          lastOffset.current !== null &&
          Math.abs(lastOffset.current - offset) <= 1;
        if (already) confirm();
        else unconfirm();
        listRef.current?.scrollToOffset({ offset, animated: false });
      } else {
        listRef.current?.scrollToEnd({ animated: false });
      }
    });
  }, [confirm, endOffset, unconfirm]);
  const stopSettling = useCallback(() => {
    settling.current = false;
    setSettled(true);
    setRevealed(true);
    if (settleTimer.current) clearTimeout(settleTimer.current);
    settleTimer.current = null;
    if (revealTimer.current) clearTimeout(revealTimer.current);
    revealTimer.current = null;
  }, []);
  useEffect(() => {
    settling.current = true;
    pinIssued.current = false;
    loaded.current = false;
    confirmed.current = false;
    lastOffset.current = null;
    setSettled(false);
    setRevealed(false);
    // A chat with nothing to measure (or a load that never reports) still has to show up.
    const fallback = setTimeout(() => setRevealed(true), 800);
    return () => {
      clearTimeout(fallback);
      stopSettling();
    };
  }, [id, stopSettling]);
  const onScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      if (Platform.OS !== "ios") return;
      lastOffset.current = e.nativeEvent.contentOffset.y;
      if (!settling.current) return;
      // Only a pin can confirm: the list's own first scroll can sit at the computed end by
      // coincidence while the content height is still unknown.
      if (!pinIssued.current) return;
      if (Math.abs(e.nativeEvent.contentOffset.y - endOffset()) <= 1) {
        confirm();
      } else {
        unconfirm();
        pinToBottom();
      }
    },
    [confirm, endOffset, pinToBottom, unconfirm],
  );
  useEffect(() => {
    unconfirm();
    pinToBottom();
  }, [headerHeight, pinToBottom, unconfirm]);
  // The transcript runs under the transparent header on iOS, so it starts below it. The insets
  // are explicit rather than iOS's automatic ones: the automatic behavior would add the home
  // indicator's safe area under the composer's inset, which already covers it, leaving a strip
  // of dead scroll past the last message that the native scroll-to-end never reaches.
  const ChatScroll = useMemo(
    () =>
      forwardRef<any, ScrollViewProps>(function ChatScroll(props, ref) {
        return (
          <KeyboardChatScrollView
            ref={ref}
            {...props}
            blankSpace={composerBlank}
            extraContentPadding={composerExtra}
            applyWorkaroundForContentInsetHitTestBug
          />
        );
      }),
    [composerBlank, composerExtra],
  );

  useEffect(() => {
    useStore.setState({ openChatId: id });
    markRead(id);
    return () => {
      if (useStore.getState().openChatId === id)
        useStore.setState({ openChatId: null });
    };
  }, [id]);

  useEffect(() => {
    if (chat && chat.unread_count > 0) markRead(id);
  }, [chat, id]);

  const rows = useMemo(
    () => (chat ? buildRows(chat, bots, workingBotIds, isWorking, status) : []),
    [chat, bots, workingBotIds, isWorking, status],
  );
  const members = useMemo(
    () =>
      (chat?.bot_ids ?? [])
        .map((b) => bots.get(b))
        .filter((b): b is Bot => !!b),
    [chat, bots],
  );
  const isGroup = chat?.kind === "group";
  const title = chat ? chatTitle(chat) : "Chat";
  // After the Mac app: "Message Chef", or the group's title with a hint that @ addresses one bot.
  const placeholder =
    !isGroup && members[0]
      ? `Message ${members[0].name}`
      : members.length > 1
        ? `Message ${title} — @ to address one bot`
        : `Message ${title}`;

  const onLongPress = useCallback((message: Message) => {
    const text = message.body.kind === "text" ? message.body.text : "";
    const copy = () => {
      void Clipboard.setStringAsync(text);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    };
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions(
        { options: ["Copy", "Cancel"], cancelButtonIndex: 1 },
        (index) => {
          if (index === 0) copy();
        },
      );
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
        <Text style={{ color: p.secondaryLabel }}>
          This chat is no longer on the roster.
        </Text>
      </View>
    );
  }

  return (
    <View style={{ flex: 1, backgroundColor: p.background }}>
      <Stack.Screen options={{ title }} />
      <Stack.Title asChild>
        <Pressable
          onPress={() => router.push(`/chat-info/${id}`)}
          style={styles.titleView}
          accessibilityLabel={`${title}, info`}
        >
          <AvatarCluster bots={members} size={30} working={isWorking} />
          <Text
            style={[styles.titleText, { color: p.label }]}
            numberOfLines={1}
          >
            {title}
          </Text>
        </Pressable>
      </Stack.Title>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button
          icon="info.circle"
          accessibilityLabel="Chat info"
          onPress={() => router.push(`/chat-info/${id}`)}
        />
      </Stack.Toolbar>
      <View style={{ flex: 1 }}>
        <View
          style={{ flex: 1, opacity: revealed ? 1 : 0 }}
          onLayout={(e) => {
            if (e.nativeEvent.layout.height !== layoutHeight.current)
              unconfirm();
            layoutHeight.current = e.nativeEvent.layout.height;
            pinToBottom();
          }}
        >
          <FlashList
            renderScrollComponent={ChatScroll}
            ref={listRef}
            data={rows}
            keyExtractor={(row) => row.key}
            getItemType={(row) => row.type}
            contentInsetAdjustmentBehavior="never"
            contentInset={{ top: topInset }}
            scrollIndicatorInsets={{ top: topInset }}
            keyboardDismissMode="interactive"
            maintainVisibleContentPosition={{
              startRenderingFromBottom: true,
              autoscrollToBottomThreshold: 0.25,
              animateAutoScrollToBottom: settled,
            }}
            contentContainerStyle={{
              paddingTop: Platform.OS === "ios" ? 0 : 8,
            }}
            onLoad={() => {
              loaded.current = true;
              pinToBottom();
              if (confirmed.current) scheduleReveal();
              // Android has no confirming scroll event: the pin is a frame away, so show then.
              if (Platform.OS !== "ios")
                setTimeout(() => setRevealed(true), 50);
              if (settleTimer.current) clearTimeout(settleTimer.current);
              settleTimer.current = setTimeout(stopSettling, 1500);
            }}
            onContentSizeChange={(_w, h) => {
              if (h !== contentHeight.current) unconfirm();
              contentHeight.current = h;
              pinToBottom();
            }}
            onScroll={onScroll}
            scrollEventThrottle={16}
            onScrollBeginDrag={stopSettling}
            renderItem={({ item }) => {
              switch (item.type) {
                case "day":
                  return <DayRow at={item.at} />;
                case "message":
                  return (
                    <MessageRow
                      row={item}
                      bots={bots}
                      isGroup={isGroup}
                      onLongPress={onLongPress}
                    />
                  );
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
        </View>
        <KeyboardStickyView
          style={[
            styles.composer,
            { paddingBottom: Math.max(insets.bottom, 8) },
          ]}
          // Open, the composer's home-indicator padding is not needed: it sits on the keys.
          offset={{ closed: 0, opened: insets.bottom }}
          onLayout={(e) => {
            const blank = e.nativeEvent.layout.height + COMPOSER_GAP;
            if (blank !== composerBlank.value) unconfirm();
            composerBlank.value = blank;
            composerExtra.value = blank - insets.bottom;
            pinToBottom();
          }}
          pointerEvents="box-none"
        >
          <Composer
            members={members}
            isGroup={isGroup}
            placeholder={placeholder}
            onSend={(text, files) => {
              engine.sendMessage(id, text, files).catch((error) => {
                Alert.alert(
                  "Could not send",
                  error instanceof Error ? error.message : String(error),
                );
              });
            }}
          />
        </KeyboardStickyView>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  missing: { flex: 1, alignItems: "center", justifyContent: "center" },
  composer: { position: "absolute", left: 0, right: 0, bottom: 0 },
  titleView: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    maxWidth: 240,
  },
  titleText: { fontSize: 17, fontWeight: "600", flexShrink: 1 },
});
