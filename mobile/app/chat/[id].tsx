import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { LinearGradient } from "expo-linear-gradient";
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
  KeyboardController,
  KeyboardStickyView,
  useKeyboardHandler,
} from "react-native-keyboard-controller";
import Animated, {
  runOnJS,
  useAnimatedReaction,
  useSharedValue,
  ZoomIn,
  ZoomOut,
} from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { chatTitle, engine } from "../../src/core/engine";
import type { Bot } from "../../src/core/model";
import { clearPushes } from "../../src/core/push";
import {
  markRead,
  useBotMap,
  useChat,
  useIsWorking,
  useStore,
  useWorkingBots,
} from "../../src/core/store";
import { t } from "../../src/i18n";
import { AvatarCluster } from "../../src/ui/Avatar";
import { Composer, Surface } from "../../src/ui/Composer";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import {
  buildRows,
  DayRow,
  MarkerRow,
  MessageRow,
  NoticeRow,
  PermissionRow,
  StatusRow,
  WorkingRow,
  type Row,
} from "../../src/ui/transcript";

/// Breathing room between the last message and the composer, as on the Mac.
const COMPOSER_GAP = 14;
/// How much taller the composer gets when it expands on focus, until it has been seen to.
const FOCUS_GROWTH_GUESS = 36;
/// How far from the end the transcript is scrolled before the jump-to-bottom disc shows.
const JUMP_DISTANCE = 160;
const JUMP_DISC = 36;
const ANDROID_BAR_HEIGHT = 56;
const ANDROID_FADE_HEIGHT = 24;

export default function ChatScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const insets = useSafeAreaInsets();
  const headerHeight = useHeaderHeight();
  const visibleHeaderHeight = Platform.OS === "android" ? insets.top + ANDROID_BAR_HEIGHT : headerHeight;
  const androidHeaderHeight = visibleHeaderHeight + ANDROID_FADE_HEIGHT;
  const androidHeaderStop = visibleHeaderHeight / androidHeaderHeight;
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
  // Focusing the composer makes it taller just as the keyboard starts to move. The scroll view
  // answers a change of `extraContentPadding` by scrolling from the offset it last saw, which at
  // that moment is the one from before the keyboard's lift, so the lift would be lost. A new
  // composer height therefore waits until the keyboard has come to rest. For the lift to cover
  // the expanded composer in one motion, the padding already counts the expanded height while
  // the keyboard is down: the compact composer's height plus the growth seen on earlier focuses.
  const composerExtraTarget = useSharedValue(
    composerGuess - insets.bottom + FOCUS_GROWTH_GUESS,
  );
  const composerActual = useRef(composerGuess - insets.bottom);
  const composerCompact = useRef(Number.POSITIVE_INFINITY);
  const focusGrowth = useRef(FOCUS_GROWTH_GUESS);
  const compactAtKeyboardStart = useRef(false);
  const publishComposerExtra = useCallback(() => {
    const actual = composerActual.current;
    const compact = composerCompact.current;
    const expanded = actual > compact + 1;
    composerExtraTarget.value = expanded
      ? actual
      : compact + focusGrowth.current;
  }, [composerExtraTarget]);
  const keyboardMoving = useSharedValue(false);
  // Sending hides the keyboard; once it is down, an anchored message is put back under the
  // header in case the keyboard's own unwinding moved it.
  const keyboardEndRef = useRef<(height: number) => void>(() => {});
  const keyboardEnded = useCallback(
    (height: number) => keyboardEndRef.current(height),
    [],
  );
  // The keyboard takes over the scroll position: the pin that holds a just-opened chat at its
  // end would otherwise scroll the keyboard's lift straight back.
  const keyboardStartRef = useRef<(height: number) => void>(() => {});
  const keyboardStarted = useCallback(
    (height: number) => keyboardStartRef.current(height),
    [],
  );
  useKeyboardHandler(
    {
      onStart: (e) => {
        "worklet";
        keyboardMoving.value = true;
        runOnJS(keyboardStarted)(e.height);
      },
      onEnd: (e) => {
        "worklet";
        keyboardMoving.value = false;
        composerExtra.value = composerExtraTarget.value;
        runOnJS(keyboardEnded)(e.height);
      },
    },
    [keyboardEnded, keyboardStarted],
  );
  useAnimatedReaction(
    () => composerExtraTarget.value,
    (extra) => {
      if (!keyboardMoving.value) composerExtra.value = extra;
    },
  );
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
  // FlashList pads a transcript shorter than the list up to the list's full height, so the rows
  // start from the bottom. That padding knows nothing of the header and composer insets, which
  // would leave a short chat scrollable by their sum. The top inset gives the padding back: it
  // shrinks by the padding, down to the point where the only offset left is the end.
  const [insetTop, setInsetTop] = useState(topInset);
  // Sending a message anchors it under the header: the space below the transcript grows until
  // the rows from that message on fill the viewport, and gives way as the reply comes in. The
  // end of the scroll range then keeps the message at the top, so nothing moves while the reply
  // streams. Once the reply outgrows the viewport the anchor lets go and the list follows its end.
  const composerSpace = useRef(composerGuess);
  const anchorKey = useRef<string | null>(null);
  const anchorTarget = useRef<number | null>(null);
  const [anchored, setAnchored] = useState(false);
  const rowsRef = useRef<Row[]>([]);
  const releaseAnchor = useCallback(() => {
    anchorKey.current = null;
    anchorTarget.current = null;
    setAnchored(false);
  }, []);
  // The space under the transcript with the keyboard down: the composer's, or the padding the
  // scroll view keeps for the expanded composer when that is the larger.
  const restingSpace = useCallback(
    () => Math.max(composerSpace.current, composerExtraTarget.value),
    [composerExtraTarget],
  );
  const blankFor = useCallback(() => {
    const list = listRef.current;
    const key = anchorKey.current;
    if (!list || !key || layoutHeight.current === 0) return restingSpace();
    const index = rowsRef.current.findIndex((row) => row.key === key);
    const layout = index < 0 ? undefined : list.getLayout(index);
    // The sent message has not reached the list yet: keep what is there.
    if (!layout) return composerBlank.value;
    const tail = list.getChildContainerDimensions().height - layout.y;
    const blank = layoutHeight.current - topInsetRef.current - tail;
    if (blank <= restingSpace()) {
      releaseAnchor();
      return restingSpace();
    }
    return blank;
  }, [composerBlank, releaseAnchor, restingSpace]);
  const topInsetFor = useCallback(() => {
    const list = listRef.current;
    if (Platform.OS !== "ios") return 0;
    if (!list || layoutHeight.current === 0 || contentHeight.current === 0)
      return topInsetRef.current;
    const padding = Math.max(
      0,
      list.getWindowSize().height -
        list.getChildContainerDimensions().height -
        list.getFirstItemOffset(),
    );
    const end =
      contentHeight.current + composerBlank.value - layoutHeight.current;
    return Math.round(
      Math.min(
        topInsetRef.current,
        Math.max(topInsetRef.current - padding, -end),
      ),
    );
  }, [composerBlank]);
  const endOffsetRef = useRef<() => number>(() => 0);
  const syncInsetTop = useCallback(() => {
    composerBlank.value = blankFor();
    setInsetTop(topInsetFor());
    if (!anchorKey.current) return;
    // The end moves only when the rows above the anchor are measured anew.
    const target = endOffsetRef.current();
    if (
      anchorTarget.current !== null &&
      Math.abs(anchorTarget.current - target) <= 1
    )
      return;
    const index = rowsRef.current.findIndex(
      (row) => row.key === anchorKey.current,
    );
    if (index < 0 || !listRef.current?.getLayout(index)) return;
    anchorTarget.current = target;
    if (Platform.OS === "ios")
      listRef.current?.scrollToOffset({ offset: target, animated: true });
    else listRef.current?.scrollToEnd({ animated: true });
  }, [blankFor, composerBlank, topInsetFor]);
  const endOffset = useCallback(
    () =>
      Math.max(
        -topInsetFor(),
        contentHeight.current + composerBlank.value - layoutHeight.current,
      ),
    [composerBlank, topInsetFor],
  );
  endOffsetRef.current = endOffset;
  // Where the transcript rests right now: with the keyboard up its end sits above the composer
  // on the keys, whatever space an anchored message holds below it.
  const keyboardHeight = useRef(0);
  const restingEnd = useCallback(() => {
    if (keyboardHeight.current === 0) return endOffset();
    return Math.max(
      -topInsetFor(),
      contentHeight.current +
        keyboardHeight.current +
        composerExtraTarget.value -
        layoutHeight.current,
    );
  }, [composerExtraTarget, endOffset, topInsetFor]);
  // The jump-to-bottom disc floats over the composer while the end is out of reach.
  const [awayFromEnd, setAwayFromEnd] = useState(false);
  const [composerHeight, setComposerHeight] = useState(
    composerGuess - COMPOSER_GAP,
  );
  const jumpToEnd = useCallback(() => {
    if (Platform.OS === "ios")
      listRef.current?.scrollToOffset({ offset: restingEnd(), animated: true });
    else listRef.current?.scrollToEnd({ animated: true });
  }, [restingEnd]);
  // The space under an anchored message swallows the keyboard: the scroll view sees room below
  // the content and lifts nothing, so the keys would cover a reply that reaches under them. With
  // the keyboard up the end of the transcript goes above the composer instead, unless everything
  // from the anchor on fits there anyway; with the keyboard down the anchor takes over again.
  const liftAnchored = (height: number) => {
    const list = listRef.current;
    const key = anchorKey.current;
    if (!list || !key || Platform.OS !== "ios") return;
    const index = rowsRef.current.findIndex((row) => row.key === key);
    const layout = index < 0 ? undefined : list.getLayout(index);
    if (!layout) return;
    const covered = height + composerExtraTarget.value;
    const tail = list.getChildContainerDimensions().height - layout.y;
    if (tail <= layoutHeight.current - topInsetRef.current - covered) return;
    list.scrollToOffset({
      offset: contentHeight.current + covered - layoutHeight.current,
      animated: true,
    });
  };
  keyboardEndRef.current = (height) => {
    keyboardHeight.current = height;
    // The composer has expanded by now: what it grew by is what the next focus will need.
    const grown = composerActual.current - composerCompact.current;
    if (height > 0 && compactAtKeyboardStart.current && grown > 1)
      focusGrowth.current = grown;
    publishComposerExtra();
    if (!anchorKey.current) return;
    if (height > 0) return liftAnchored(height);
    anchorTarget.current = null;
    syncInsetTop();
  };
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
  keyboardStartRef.current = (height) => {
    compactAtKeyboardStart.current =
      composerActual.current <= composerCompact.current + 1;
    if (settling.current) stopSettling();
    if (height > 0) liftAnchored(height);
  };
  useEffect(() => {
    settling.current = true;
    pinIssued.current = false;
    loaded.current = false;
    confirmed.current = false;
    lastOffset.current = null;
    releaseAnchor();
    setSettled(false);
    setRevealed(false);
    setAwayFromEnd(false);
    // A chat with nothing to measure (or a load that never reports) still has to show up.
    const fallback = setTimeout(() => setRevealed(true), 800);
    return () => {
      clearTimeout(fallback);
      stopSettling();
    };
  }, [id, releaseAnchor, stopSettling]);
  const onScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
      const distance =
        Platform.OS === "ios"
          ? restingEnd() - contentOffset.y
          : contentSize.height - layoutMeasurement.height - contentOffset.y;
      setAwayFromEnd(!settling.current && distance > JUMP_DISTANCE);
      if (Platform.OS !== "ios") return;
      lastOffset.current = contentOffset.y;
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
    [confirm, endOffset, pinToBottom, restingEnd, unconfirm],
  );
  useEffect(() => {
    unconfirm();
    syncInsetTop();
    pinToBottom();
  }, [anchored, headerHeight, insetTop, pinToBottom, syncInsetTop, unconfirm]);
  // The transcript runs under the transparent header on both platforms. On iOS the insets are
  // explicit rather than automatic: the automatic behavior would add the home
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

  // FlashList scrolls to the end whenever `data` changes identity while the list sits near the
  // bottom, animated once settled. Everything it is handed is therefore kept stable across
  // renders: the state flip on the first drag (stopSettling) re-renders this screen, and a fresh
  // rows array or renderItem there would launch an animated scroll-to-end against the drag.
  const contentInset = useMemo(() => ({ top: insetTop }), [insetTop]);
  // The scroll view ends the bar's track where its bottom padding starts, which includes the
  // gap kept above the composer; the bar itself runs down to the composer.
  const scrollIndicatorInsets = useMemo(
    () => ({ top: topInset, bottom: -COMPOSER_GAP }),
    [topInset],
  );
  const maintainVisibleContentPosition = useMemo(
    () => ({
      startRenderingFromBottom: true,
      // An anchored message holds the end still; FlashList's own catch-up would race the
      // shrinking space below it.
      autoscrollToBottomThreshold: anchored ? -1 : 0.25,
      animateAutoScrollToBottom: settled,
    }),
    [anchored, settled],
  );

  useEffect(() => {
    useStore.setState({ openChatId: id });
    markRead(id);
    void clearPushes(id);
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
  rowsRef.current = rows;
  const members = useMemo(
    () =>
      (chat?.bot_ids ?? [])
        .map((b) => bots.get(b))
        .filter((b): b is Bot => !!b),
    [chat, bots],
  );
  const isGroup = chat?.kind === "group";
  const title = chat ? chatTitle(chat) : t("Chat");
  // After the Mac app: "Message Chef", or the group's title with a hint that @ addresses one bot.
  const placeholder =
    !isGroup && members[0]
      ? t("Message {name}", { name: members[0].name })
      : members.length > 1
        ? t("Message {title} — @ to address one bot", { title })
        : t("Message {name}", { name: title });

  /// The whole message behind a "Messaged ◉ Name" marker, as a sheet.
  const openMarker = useCallback(
    (row: Extract<Row, { type: "marker" }>) => {
      const title = `${row.text} ${row.bot?.name ?? t("a teammate")}`;
      router.push({
        pathname: "/message/[id]",
        params: { id: row.key, chat: id, title },
      });
    },
    [router, id],
  );

  const renderItem = useCallback(
    ({ item }: { item: Row }) => {
      switch (item.type) {
        case "day":
          return <DayRow at={item.at} />;
        case "message":
          return <MessageRow row={item} bots={bots} isGroup={isGroup} />;
        case "marker":
          return <MarkerRow row={item} onPress={openMarker} />;
        case "notice":
          return <NoticeRow row={item} />;
        case "permission":
          return (
            <PermissionRow
              row={item}
              onDecide={(decision) =>
                engine.answerPermission(
                  item.message.chat_id,
                  item.message.id,
                  decision,
                )
              }
            />
          );
        case "working":
          return <WorkingRow bots={item.bots} isGroup={isGroup} />;
        case "status":
          return <StatusRow text={item.text} />;
      }
    },
    [bots, isGroup, openMarker],
  );

  if (!chat) {
    return (
      <View style={styles.missing}>
        <Stack.Screen options={{ title: t("Chat") }} />
        <Text style={{ color: p.secondaryLabel }}>
          {t("This chat is no longer on the roster.")}
        </Text>
      </View>
    );
  }

  return (
    <View style={{ flex: 1, backgroundColor: p.background }}>
      <Stack.Screen options={{ title }} />
      {Platform.OS === "ios" ? (
        <>
          <Stack.Title asChild>
            <Pressable
              onPress={() => router.push(`/chat-info/${id}`)}
              style={styles.titleView}
              accessibilityLabel={t("{title}, info", { title })}
            >
              <AvatarCluster bots={members} size={30} working={isWorking} />
              <Text style={[styles.titleText, { color: p.label }]} numberOfLines={1}>
                {title}
              </Text>
            </Pressable>
          </Stack.Title>
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Button icon="ellipsis" accessibilityLabel={t("Chat info")} onPress={() => router.push(`/chat-info/${id}`)} />
          </Stack.Toolbar>
        </>
      ) : (
        <View pointerEvents="box-none" style={[styles.androidHeader, { height: androidHeaderHeight }]}>
          <LinearGradient
            pointerEvents="none"
            colors={
              p.dark
                ? ["rgba(10,10,12,0.97)", "rgba(10,10,12,0.86)", "rgba(10,10,12,0.68)", "rgba(10,10,12,0)"]
                : ["rgba(255,255,255,0.97)", "rgba(255,255,255,0.86)", "rgba(255,255,255,0.68)", "rgba(255,255,255,0)"]
            }
            locations={[0, androidHeaderStop * 0.55, androidHeaderStop, 1]}
            start={{ x: 0.5, y: 0 }}
            end={{ x: 0.5, y: 1 }}
            style={StyleSheet.absoluteFill}
          />
          <View style={[styles.androidHeaderControls, { height: visibleHeaderHeight, paddingTop: insets.top }]}>
            <Pressable onPress={() => router.back()} style={styles.androidHeaderButton} accessibilityRole="button" accessibilityLabel={t("Back")}>
              <Symbol name="arrow.left" size={26} color={p.label} />
            </Pressable>
            <Pressable
              onPress={() => router.push(`/chat-info/${id}`)}
              style={styles.androidHeaderTitle}
              accessibilityRole="button"
              accessibilityLabel={t("{title}, info", { title })}
            >
              <AvatarCluster bots={members} size={30} working={isWorking} />
              <Text style={[styles.titleText, { color: p.label }]} numberOfLines={1}>
                {title}
              </Text>
            </Pressable>
            <Pressable onPress={() => router.push(`/chat-info/${id}`)} style={styles.androidHeaderButton} accessibilityRole="button" accessibilityLabel={t("Chat info")}>
              <Symbol name="ellipsis" size={24} color={p.label} />
            </Pressable>
          </View>
        </View>
      )}
      <View style={{ flex: 1 }}>
        <View
          style={{ flex: 1, opacity: revealed ? 1 : 0 }}
          onLayout={(e) => {
            if (e.nativeEvent.layout.height !== layoutHeight.current)
              unconfirm();
            layoutHeight.current = e.nativeEvent.layout.height;
            syncInsetTop();
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
            contentInset={contentInset}
            // The insets here already account for the header and the composer; iOS would add the
            // safe area to the scroll bar's track again and keep it short of both ends.
            automaticallyAdjustsScrollIndicatorInsets={false}
            scrollIndicatorInsets={scrollIndicatorInsets}
            onCommitLayoutEffect={syncInsetTop}
            keyboardDismissMode="interactive"
            maintainVisibleContentPosition={maintainVisibleContentPosition}
            contentContainerStyle={{
              // Android's chat bar is transparent too: the first rows begin below it, then
              // naturally scroll beneath it. UIKit gets the same spacing from contentInset.
              paddingTop: Platform.OS === "ios" ? 0 : visibleHeaderHeight + 8,
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
              syncInsetTop();
              pinToBottom();
            }}
            // Nearing the first message: the page before it. Not while the list is still
            // finding the end after opening.
            onStartReached={() => {
              if (!settling.current) void engine.loadOlder(id);
            }}
            onStartReachedThreshold={1}
            onScroll={onScroll}
            scrollEventThrottle={16}
            onScrollBeginDrag={stopSettling}
            renderItem={renderItem}
          />
        </View>
        <KeyboardStickyView
          style={[styles.jump, { bottom: composerHeight + 10 }]}
          offset={{ closed: 0, opened: insets.bottom }}
          pointerEvents="box-none"
        >
          {awayFromEnd && (
            <Animated.View
              entering={ZoomIn.duration(160)}
              exiting={ZoomOut.duration(160)}
            >
              <Pressable
                onPress={jumpToEnd}
                hitSlop={8}
                accessibilityRole="button"
                accessibilityLabel={t("Jump to bottom")}
              >
                <Surface
                  style={styles.jumpDisc}
                  tint={p.cell}
                  edge={p.dark ? "rgba(255,255,255,0.14)" : "rgba(0,0,0,0.1)"}
                >
                  <Symbol name="arrow.down" size={16} color={p.label} />
                </Surface>
              </Pressable>
            </Animated.View>
          )}
        </KeyboardStickyView>
        <KeyboardStickyView
          style={[
            styles.composer,
            { paddingBottom: Math.max(insets.bottom, 8) },
          ]}
          // Open, the composer's home-indicator padding is not needed: it sits on the keys.
          offset={{ closed: 0, opened: insets.bottom }}
          onLayout={(e) => {
            setComposerHeight(e.nativeEvent.layout.height);
            const blank = e.nativeEvent.layout.height + COMPOSER_GAP;
            if (blank !== composerSpace.current) unconfirm();
            composerSpace.current = blank;
            const extra = blank - insets.bottom;
            composerActual.current = extra;
            composerCompact.current = Math.min(composerCompact.current, extra);
            publishComposerExtra();
            syncInsetTop();
            pinToBottom();
          }}
          pointerEvents="box-none"
        >
          <Composer
            members={members}
            isGroup={isGroup}
            placeholder={placeholder}
            onSend={(text, files) => {
              // The anchor is measured against the screen without the keyboard.
              void KeyboardController.dismiss();
              const sent = engine
                .sendMessage(id, text, files)
                .then((message) => {
                  stopSettling();
                  anchorKey.current = message.id;
                  anchorTarget.current = null;
                  setAnchored(true);
                });
              sent.catch((error) => {
                Alert.alert(
                  t("Could not send"),
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
  androidHeader: { position: "absolute", top: 0, left: 0, right: 0, zIndex: 100 },
  androidHeaderControls: { flexDirection: "row", alignItems: "center", paddingHorizontal: 12 },
  androidHeaderButton: { width: 48, height: ANDROID_BAR_HEIGHT, alignItems: "center", justifyContent: "center" },
  androidHeaderTitle: { flex: 1, height: ANDROID_BAR_HEIGHT, flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, paddingHorizontal: 8 },
  composer: { position: "absolute", left: 0, right: 0, bottom: 0 },
  jump: { position: "absolute", right: 12 },
  jumpDisc: {
    width: JUMP_DISC,
    height: JUMP_DISC,
    borderRadius: JUMP_DISC / 2,
    alignItems: "center",
    justifyContent: "center",
  },
  titleView: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    maxWidth: 240,
  },
  titleText: { fontSize: 17, fontWeight: "600", flexShrink: 1 },
});
