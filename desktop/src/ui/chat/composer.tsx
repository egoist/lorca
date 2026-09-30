// The composer, after the macOS app's ComposerView: a pill floating over the transcript with the
// + button, the text, and Stop and Send. One line of text sits beside the controls; longer text
// or attachments open it up, the chips above the text and the controls in a row under it.
// Return sends (Shift-Return breaks the line) unless Settings reserves sending for Ctrl-Return.
// `@` offers the bots, and a name picked from the menu goes out with the message by id.

import { createMemo, createSignal, For, onSettled, Show } from "solid-js";
import { files, hostInfo, onDroppedFiles, preferences, type FileInfo } from "../../host";
import { L } from "../../l10n";
import { isImage, sizeText, type Bot } from "../../model/models";
import { attachmentID, store, type OutgoingAttachment } from "../../model/store";
import { Avatar, botAvatar } from "../avatar";
import { shortcutText } from "../commands";
import { Icon } from "../icons";
import { alert } from "../overlay";

const maxBytes = 100 * 1024 * 1024;
const maxCount = 10;
const maxTextHeight = 168;

export interface ComposerHandle {
  focus(): void;
  setText(text: string): void;
}

/** Where the caret sits in a textarea, in viewport pixels: the top of its line and its x. */
function caretPoint(area: HTMLTextAreaElement, position: number): { x: number; top: number; bottom: number } {
  const mirror = document.createElement("div");
  const style = getComputedStyle(area);
  for (const property of ["font", "letter-spacing", "padding", "border", "box-sizing", "line-height", "white-space", "word-wrap", "overflow-wrap", "tab-size", "text-indent"]) {
    mirror.style.setProperty(property, style.getPropertyValue(property));
  }
  mirror.style.position = "fixed";
  mirror.style.visibility = "hidden";
  mirror.style.whiteSpace = "pre-wrap";
  mirror.style.width = `${area.clientWidth}px`;
  const rect = area.getBoundingClientRect();
  mirror.style.left = `${rect.left}px`;
  mirror.style.top = `${rect.top - area.scrollTop}px`;
  mirror.textContent = area.value.slice(0, position);
  const marker = document.createElement("span");
  marker.textContent = "​";
  mirror.appendChild(marker);
  document.body.appendChild(mirror);
  const at = marker.getBoundingClientRect();
  mirror.remove();
  return { x: at.left, top: at.top, bottom: at.bottom };
}

function outgoing(file: FileInfo): OutgoingAttachment {
  return {
    attachment: { id: attachmentID(), name: file.name, mime: file.mime || "application/octet-stream", size: file.size, width: file.width, height: file.height },
    path: file.path,
    url: file.url,
  };
}

export function Composer(props: {
  placeholder: string;
  bots: Bot[];
  isResponding: boolean;
  onSend: (text: string, attachments: OutgoingAttachment[], mentions: string[]) => void;
  onStop: () => void;
  ref?: (handle: ComposerHandle) => void;
  onHeight?: (height: number) => void;
}) {
  const [text, setText] = createSignal("");
  const [attachments, setAttachments] = createSignal<OutgoingAttachment[]>([]);
  const [expanded, setExpanded] = createSignal(false);
  const [mention, setMention] = createSignal<{ start: number; end: number; bots: Bot[]; x: number; top: number; bottom: number } | null>(null);
  const [mentionIndex, setMentionIndex] = createSignal(0);
  let picked: Bot[] = [];
  let area: HTMLTextAreaElement | undefined;
  let backdrop: HTMLDivElement | undefined;
  let field: HTMLDivElement | undefined;
  let root: HTMLDivElement | undefined;

  const hasContent = () => text().trim() !== "" || attachments().length > 0;

  const focus = () => area?.focus();
  const replaceText = (value: string) => {
    setText(value);
    picked = [];
    if (area) area.value = value;
    queueMicrotask(() => {
      updateLayout();
      if (area) area.setSelectionRange(value.length, value.length);
    });
  };
  props.ref?.({ focus, setText: replaceText });

  /** One line beside the controls unless the text needs more room: a newline, a wrap, or files. */
  const updateLayout = () => {
    if (!area || !field) return;
    const style = getComputedStyle(area);
    const line = parseFloat(style.lineHeight) || 19;
    const value = area.value;
    let wants = attachments().length > 0 || value.includes("\n");
    if (!wants) {
      const measure = document.createElement("span");
      measure.style.font = style.font;
      measure.style.whiteSpace = "pre";
      measure.style.position = "fixed";
      measure.style.visibility = "hidden";
      measure.textContent = value;
      document.body.appendChild(measure);
      const width = measure.getBoundingClientRect().width;
      measure.remove();
      const compactWidth = field.clientWidth - 16 - 28 - 16 - trailingWidth() - 12;
      wants = expanded() ? width > compactWidth - 12 : width > compactWidth;
    }
    if (wants !== expanded()) setExpanded(wants);
    queueMicrotask(() => {
      if (!area) return;
      area.style.height = "0px";
      const height = Math.min(maxTextHeight, Math.max(line + 8, area.scrollHeight));
      area.style.height = `${height}px`;
      area.style.overflowY = area.scrollHeight > maxTextHeight ? "auto" : "hidden";
      if (backdrop) backdrop.style.height = `${height}px`;
      syncScroll();
    });
  };

  const trailingWidth = () => (props.isResponding ? 32 : 0) + 28;

  const syncScroll = () => {
    if (area && backdrop) backdrop.scrollTop = area.scrollTop;
  };

  // MARK: Mentions

  /** The `@token` the caret sits in, if any: an `@` at the start or after whitespace, and no space
   * before the caret. */
  const mentionRange = (): { start: number; end: number } | null => {
    if (!area) return null;
    const value = area.value;
    const caret = area.selectionStart;
    if (caret !== area.selectionEnd || caret === 0) return null;
    for (let index = caret - 1; index >= 0; index--) {
      const character = value[index]!;
      if (character === "@") {
        const isStart = index === 0 || /\s/.test(value[index - 1]!);
        return isStart ? { start: index, end: caret } : null;
      }
      if (character === " " || character === "\n") return null;
      if (caret - index > 24) return null;
    }
    return null;
  };

  const updateMentions = () => {
    const range = mentionRange();
    if (!range || !area || props.bots.length === 0) {
      setMention(null);
      return;
    }
    const query = area.value.slice(range.start + 1, range.end).toLowerCase();
    const matches = props.bots.filter((bot) => query === "" || bot.name.toLowerCase().startsWith(query));
    if (matches.length === 0) {
      setMention(null);
      return;
    }
    const point = caretPoint(area, range.start);
    if (mentionIndex() >= matches.length) setMentionIndex(0);
    setMention({ ...range, bots: matches, ...point });
  };

  const insertMention = (bot: Bot) => {
    const range = mentionRange();
    if (!range || !area) return;
    const insertion = `@${bot.name} `;
    area.setRangeText(insertion, range.start, range.end, "end");
    setText(area.value);
    picked.push(bot);
    setMention(null);
    updateLayout();
  };

  // MARK: Attachments

  /** Adds the files it can; the rest get one alert. */
  const addFiles = (infos: FileInfo[]) => {
    const problems: string[] = [];
    const added: OutgoingAttachment[] = [];
    for (const info of infos) {
      if (attachments().length + added.length >= maxCount) {
        problems.push(L("At most %d files per message.", maxCount));
        break;
      }
      if (!info.isFile) problems.push(L("%@ is not a file.", info.name));
      else if (info.size > maxBytes) problems.push(L("%@ is larger than %d MB.", info.name, maxBytes / 1024 / 1024));
      else added.push(outgoing(info));
    }
    if (added.length > 0) {
      setAttachments([...attachments(), ...added]);
      updateLayout();
    }
    if (problems.length > 0) void alert({ message: L("Some files were not attached"), informative: problems.join("\n") });
  };

  const attach = async () => {
    const chosen = await files.choose({ multiple: true, message: L("Attach files to your message"), buttonLabel: L("Attach") });
    if (chosen.length > 0) addFiles(chosen);
    focus();
  };

  const onPaste = async (event: ClipboardEvent) => {
    const data = event.clipboardData;
    if (!data) return;
    const pasted = [...data.files];
    // An image with no text beside it (a screenshot, a copied picture) is an attachment, and so
    // are copied files.
    const hasText = data.types.includes("text/plain") && data.getData("text/plain") !== "";
    if (pasted.length === 0 || (hasText && pasted.every((file) => file.type.startsWith("image/")))) return;
    event.preventDefault();
    const saved: FileInfo[] = [];
    for (const file of pasted) {
      try {
        saved.push(await files.savePasted(file.name, file));
      } catch {}
    }
    if (saved.length > 0) addFiles(saved);
  };

  onSettled(() => {
    const offDrop = onDroppedFiles(async ({ paths, x, y }) => {
      const target = document.elementFromPoint(x, y);
      if (!root?.parentElement?.contains(target)) return;
      addFiles(await files.inspect(paths));
      focus();
    });
    const observer = new ResizeObserver(() => props.onHeight?.(root?.offsetHeight ?? 0));
    if (root) observer.observe(root);
    const onResize = () => updateLayout();
    window.addEventListener("resize", onResize);
    updateLayout();
    return () => {
      offDrop();
      observer.disconnect();
      window.removeEventListener("resize", onResize);
    };
  });

  // MARK: Send

  const send = () => {
    if (!hasContent()) return;
    const value = text().trim();
    // A pick counts while its `@Name` is still in the text.
    const lowered = value.toLowerCase();
    const mentions = picked.filter((bot) => lowered.includes(`@${bot.name.toLowerCase()}`)).map((bot) => bot.id);
    const sending = attachments();
    picked = [];
    setAttachments([]);
    setMention(null);
    replaceText("");
    props.onSend(value, sending, mentions);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.isComposing) return;
    const panel = mention();
    if (panel) {
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        const delta = event.key === "ArrowUp" ? -1 : 1;
        setMentionIndex((index) => (index + delta + panel.bots.length) % panel.bots.length);
        return;
      }
      if (event.key === "Tab" || event.key === "Enter") {
        event.preventDefault();
        const bot = panel.bots[mentionIndex()];
        if (bot) insertMention(bot);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setMention(null);
        return;
      }
    }
    if (event.key !== "Enter") return;
    const primary = hostInfo().platform === "darwin" ? event.metaKey : event.ctrlKey;
    if (preferences().sendOnReturn) {
      if (event.shiftKey) return;
      event.preventDefault();
      send();
    } else if (primary) {
      event.preventDefault();
      send();
    }
  };

  /** The text with each `@Name` tinted, behind the transparent field, so mentions read as addressed. */
  const highlighted = createMemo(() => {
    const value = text();
    const names = props.bots.map((bot) => ({ bot, needle: `@${bot.name}`.toLowerCase() }));
    const parts: { text: string; bot?: Bot }[] = [];
    const lowered = value.toLowerCase();
    let index = 0;
    let plain = "";
    while (index < value.length) {
      const hit = names.find(({ needle }) => lowered.startsWith(needle, index));
      if (hit) {
        if (plain) parts.push({ text: plain });
        plain = "";
        parts.push({ text: value.slice(index, index + hit.needle.length), bot: hit.bot });
        index += hit.needle.length;
      } else {
        plain += value[index];
        index++;
      }
    }
    if (plain) parts.push({ text: plain });
    return parts;
  });

  const sendTooltip = () =>
    (preferences().sendOnReturn ? L("Send (Return) · Shift-Return for a new line") : L("Send (%@)", shortcutText("CmdOrCtrl+Enter"))) +
    (props.bots.length > 1 ? L(" · @ to mention") : "");

  return (
    <div class="composer" ref={(el) => (root = el)}>
      <div ref={(el) => (field = el)} class={["composer-field", { expanded: expanded() }]} onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          event.preventDefault();
          focus();
        }
      }}>
        <Show when={attachments().length > 0}>
          <div class="composer-strip">
            <For each={attachments()}>
              {(item) => (
                <div class={["composer-chip", isImage(item.attachment) ? "image" : "file"]} title={item.attachment.name}>
                  <Show
                    when={isImage(item.attachment) && item.url}
                    fallback={
                      <div class="composer-chip-card">
                        <Icon name="doc.fill" size={18} strokeWidth={1.8} />
                        <span class="composer-chip-text">
                          <span class="truncate">{item.attachment.name}</span>
                          <span class="composer-chip-size">{sizeText(item.attachment.size)}</span>
                        </span>
                      </div>
                    }
                  >
                    <img src={item.url} alt="" draggable={false} />
                  </Show>
                  <button
                    class="composer-chip-remove"
                    title={L("Remove")}
                    aria-label={L("Remove")}
                    onClick={() => {
                      setAttachments(attachments().filter((other) => other !== item));
                      updateLayout();
                      focus();
                    }}
                  >
                    <Icon name="xmark" size={9} strokeWidth={3.2} />
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
        <button class="composer-button secondary composer-attach" title={L("Attach files")} aria-label={L("Attach files")} onClick={() => void attach()}>
          <Icon name="plus" size={16} strokeWidth={2.2} />
        </button>
        <div class="composer-text">
          <div class="composer-backdrop" ref={(el) => (backdrop = el)} aria-hidden="true">
            <For each={highlighted()}>{(part) => (part.bot ? <span style={{ color: `var(--${part.bot.accent})`, "font-weight": 600 }}>{part.text}</span> : part.text)}</For>
            {"​"}
          </div>
          <textarea
            ref={(el) => (area = el)}
            class="composer-input"
            rows={1}
            placeholder={props.placeholder}
            aria-label={props.placeholder}
            spellcheck={true}
            onInput={(event) => {
              setText(event.currentTarget.value);
              updateLayout();
              updateMentions();
            }}
            onKeyDown={onKeyDown}
            onKeyUp={(event) => {
              if (event.key.startsWith("Arrow") && !mention()) updateMentions();
            }}
            onClick={updateMentions}
            onScroll={syncScroll}
            onBlur={() => setTimeout(() => setMention(null), 120)}
            onPaste={(event) => void onPaste(event)}
          />
        </div>
        <div class="composer-trailing">
          <Show when={props.isResponding}>
            <button
              class={["composer-button", hasContent() ? "secondary" : "primary"]}
              title={L("Stop responding (%@)", shortcutText("CmdOrCtrl+."))}
              aria-label={L("Stop responding")}
              onClick={() => props.onStop()}
            >
              <Icon name="stop.fill" size={11} strokeWidth={2.4} />
            </button>
          </Show>
          <Show when={hasContent() || !props.isResponding}>
            <button class="composer-button primary" title={sendTooltip()} aria-label={L("Send")} disabled={!hasContent()} onClick={send}>
              <Icon name="arrow.up" size={15} strokeWidth={2.6} />
            </button>
          </Show>
        </div>
      </div>
      <Show when={mention()}>
        {(panel) => (
          <div
            class="mention-panel"
            style={{ left: `${Math.max(8, panel().x - 10)}px`, top: `${panel().top - 8 - Math.min(panel().bots.length, 5) * 38 - 10}px` }}
            onMouseDown={(event) => event.preventDefault()}
          >
            <For each={panel().bots}>
              {(bot, index) => (
                <div class={["mention-row", { highlighted: index() === mentionIndex() }]} onMouseEnter={() => setMentionIndex(index())} onClick={() => insertMention(bot)}>
                  <Avatar content={botAvatar(bot)} size={22} />
                  <span class="mention-text">
                    <span class="mention-name truncate">{bot.name}</span>
                    <span class="mention-detail truncate">
                      {store.device(bot.runnerID)?.name ? L("on %@", store.device(bot.runnerID)!.name) : bot.provider}
                    </span>
                  </span>
                </div>
              )}
            </For>
          </div>
        )}
      </Show>
    </div>
  );
}
