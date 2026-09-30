// How a bot looks, after the macOS app's BotLookViewController: a symbol on an accent gradient, or an
// image of the user's own. The image wins while it is set; the symbol and accent stay underneath
// for when it is removed and for Devices that have not fetched it yet.

import { createSignal, For, Show } from "solid-js";
import { files, type FileInfo } from "../../host";
import { L } from "../../l10n";
import { accents, type Accent } from "../../model/models";
import { store } from "../../model/store";
import { Avatar, botAvatar, type AvatarContent } from "../avatar";
import { Button } from "../controls";
import { Icon } from "../icons";
import { alert, presentSheet, Sheet } from "../overlay";

/** The symbols offered, the same list the phone app shows. */
export const lookSymbols = [
  "sparkles",
  "wand.and.stars",
  "hammer.fill",
  "book.fill",
  "paintbrush.fill",
  "chart.bar.fill",
  "terminal.fill",
  "globe",
  "brain.head.profile",
  "magnifyingglass",
  "envelope.fill",
  "calendar",
  "flask.fill",
  "bolt.fill",
  "leaf.fill",
  "shield.fill",
  "binoculars.fill",
  "chevron.left.forwardslash.chevron.right",
  "pencil.and.scribble",
  "bolt.horizontal.fill",
  "flame.fill",
];

/** Longest side of a stored profile image, as `Files.PrepareAvatar` makes it. */
const imageSide = 512;

export function accentTitle(accent: Accent): string {
  switch (accent) {
    case "indigo":
      return L("Indigo");
    case "blue":
      return L("Blue");
    case "teal":
      return L("Teal");
    case "green":
      return L("Green");
    case "orange":
      return L("Orange");
    case "pink":
      return L("Pink");
    case "purple":
      return L("Purple");
    case "red":
      return L("Red");
  }
}

type ImageChange = { kind: "keep" } | { kind: "remove" } | { kind: "set"; file: FileInfo };

export function presentBotLook(botID: string): void {
  presentSheet((dismiss) => <BotLookSheet botID={botID} dismiss={dismiss} />);
}

function BotLookSheet(props: { botID: string; dismiss: () => void }) {
  const initial = store.bot(props.botID);
  const [symbolName, setSymbolName] = createSignal(initial?.symbolName ?? "sparkles");
  const [accent, setAccent] = createSignal<Accent>(initial?.accent ?? "indigo");
  const [imageChange, setImageChange] = createSignal<ImageChange>({ kind: "keep" });

  /** Whether the saved look, with the pending change applied, has an image. */
  const hasImage = () => {
    const change = imageChange();
    if (change.kind === "keep") return store.bot(props.botID)?.avatar !== undefined;
    return change.kind === "set";
  };

  const preview = (): AvatarContent => {
    const change = imageChange();
    if (change.kind === "set") return { kind: "image", url: change.file.url };
    if (change.kind === "keep") {
      const bot = store.bot(props.botID);
      const saved = bot ? botAvatar(bot) : undefined;
      if (saved?.kind === "image") return saved;
    }
    return { kind: "bot", symbolName: symbolName(), accent: accent() };
  };

  const chooseImage = async () => {
    const [picked] = await files.choose({ images: true, message: L("Choose an image for this bot.") });
    if (!picked) return;
    try {
      setImageChange({ kind: "set", file: await files.prepareAvatar(picked.path) });
    } catch {
      void alert({ message: L("That file could not be read as an image.") });
    }
  };

  const save = () => {
    const bot = store.bot(props.botID);
    if (bot) {
      if (symbolName() !== bot.symbolName || accent() !== bot.accent) store.setBotLook(bot.id, symbolName(), accent());
      const change = imageChange();
      if (change.kind === "remove") store.setBotAvatar(bot.id, null);
      else if (change.kind === "set") store.setBotAvatar(bot.id, change.file);
    }
    props.dismiss();
  };

  return (
    <Sheet
      title={L("Look")}
      subtitle={L("Pick a symbol and a color, or use an image of your own. Paired Devices see the same look.")}
      width={400}
      confirm={L("Save")}
      onConfirm={save}
      onCancel={props.dismiss}
      class="look-sheet"
    >
      <div class="look-preview">
        <Avatar content={preview()} size={72} />
      </div>
      <div class="look-heading">{L("Symbol").toUpperCase()}</div>
      <div class="look-symbols">
        <For each={lookSymbols}>
          {(name) => (
            <button
              class={["look-symbol", { selected: name === symbolName() }]}
              style={name === symbolName() ? { background: `var(--${accent()})` } : undefined}
              title={name}
              aria-label={name}
              aria-pressed={name === symbolName() ? "true" : "false"}
              onClick={() => setSymbolName(name)}
            >
              <Icon name={name} size={16} strokeWidth={2.2} />
            </button>
          )}
        </For>
      </div>
      <div class="look-heading">{L("Color").toUpperCase()}</div>
      <div class="look-accents">
        <For each={accents}>
          {(each) => (
            <button
              class={["look-accent", { selected: each === accent() }]}
              style={{ background: `var(--${each})` }}
              title={accentTitle(each)}
              aria-label={accentTitle(each)}
              aria-pressed={each === accent() ? "true" : "false"}
              onClick={() => setAccent(each)}
            />
          )}
        </For>
      </div>
      <div class="look-heading">{L("Image").toUpperCase()}</div>
      <div class="look-image-buttons">
        <Button onClick={() => void chooseImage()}>{L("Choose Image…")}</Button>
        <Show when={hasImage()}>
          <Button onClick={() => setImageChange({ kind: "remove" })}>{L("Remove Image")}</Button>
        </Show>
      </div>
      <div class="look-caption">
        {hasImage()
          ? L("The image shows in place of the symbol and color. It is resized to %d px and shared encrypted, like an attachment.", imageSide)
          : L("Images are resized to %d px and shared encrypted, like an attachment.", imageSide)}
      </div>
    </Sheet>
  );
}
