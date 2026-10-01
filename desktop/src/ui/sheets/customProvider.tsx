// Adds or edits a custom provider, after the macOS app's CustomProviderViewController: any server
// that speaks OpenAI's Chat Completions or Responses API, or Anthropic's Messages API. The sheet
// loads the models the server lists as the user fills it in, and the user picks the ones bots can
// use, adding any the server does not list. The CLI checks the server again before saving, and the
// provider reaches every paired Device encrypted with the account key.

import { createSignal, flush, For, onCleanup, onSettled, Show } from "solid-js";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import {
  addCandidate,
  addModel,
  customAPIs,
  customAPITitle,
  customBaseURLPlaceholder,
  customEndpointNote,
  customPresets,
  filterModels,
  isCustomKind,
  isUsableBaseURL,
  matchingPreset,
  modelDisplayName,
  orderedModelIDs,
  presetProvider,
  providerName,
  savedChecklist,
  suggestedProviderName,
  takeListing,
  toggleModel,
  type CustomAPI,
  type CustomPreset,
  type CustomProviderKind,
  type ProviderCredential,
} from "../../model/models";
import { errorText, store } from "../../model/store";
import { Button, PopUpButton, SearchField, Spinner, TextField } from "../controls";
import { Icon } from "../icons";
import { popupMenu, separator, type MenuEntry } from "../menu";
import { alert, presentSheet, Sheet } from "../overlay";
import { APIKeyField } from "./apiKeyField";

let fetching = false;

/** Add Provider…'s menu, under its button: the servers people often add, then any other. A preset
 * the account has a provider for, by name, is checked and opens that provider. */
export async function presentAddProviderMenu(anchor: HTMLElement): Promise<void> {
  const entries: MenuEntry[] = [];
  customPresets.forEach((preset, index) => {
    if (index > 0 && preset.local !== customPresets[index - 1]!.local) entries.push(separator);
    entries.push({ id: preset.name, label: preset.name, checked: presetProvider(preset, store.providers) !== undefined });
  });
  entries.push(separator, { id: "other", label: L("Other Server…") });
  const picked = await popupMenu(entries, anchor);
  if (picked === null) return;
  const preset = customPresets.find((each) => each.name === picked);
  const existing = preset ? presetProvider(preset, store.providers) : undefined;
  await presentCustomProvider(existing && isCustomKind(existing.kind) ? existing.kind : undefined, { preset });
}

/** Opens a provider the account has (its key fetched first, so the sheet opens filled in), a
 * preset, or an empty sheet. A kind the account no longer has opens the sheet to add one.
 * `onSave` gets the provider's kind once it is saved. */
export async function presentCustomProvider(
  kind?: CustomProviderKind,
  options: { preset?: CustomPreset; onSave?: (kind: CustomProviderKind) => void } = {},
): Promise<void> {
  if (fetching) return;
  fetching = true;
  const existing = kind === undefined ? undefined : store.credential(kind);
  let apiKey = "";
  try {
    if (existing) apiKey = (await store.providerAPIKey(existing.kind)).api_key ?? "";
  } catch (error) {
    fetching = false;
    void alert({ message: errorText(error) });
    return;
  }
  fetching = false;
  presentSheet((dismiss) => (
    <CustomProviderSheet existing={existing} preset={existing ? undefined : options.preset} apiKey={apiKey} onSave={options.onSave ?? (() => {})} dismiss={dismiss} />
  ));
}

/** Where the server's model list stands. */
type Listing = { kind: "needsURL" } | { kind: "loading" } | { kind: "listed" } | { kind: "unlisted" } | { kind: "failed"; message: string };

function CustomProviderSheet(props: {
  existing?: ProviderCredential;
  preset?: CustomPreset;
  apiKey: string;
  onSave: (kind: CustomProviderKind) => void;
  dismiss: () => void;
}) {
  const existing = props.existing;
  const preset = props.preset;
  /** The provider being edited; none adds one. */
  const kind = existing && isCustomKind(existing.kind) ? existing.kind : undefined;
  const initialName = existing?.name ?? preset?.name ?? "";
  const initialBaseURL = existing?.baseURL ?? preset?.baseURL ?? "";
  const [name, setName] = createSignal(initialName);
  const [api, setAPI] = createSignal<CustomAPI>(existing?.api ?? preset?.api ?? "chat-completions");
  const [baseURL, setBaseURL] = createSignal(initialBaseURL);
  const [key, setKey] = createSignal(props.apiKey);
  // A provider being edited starts with its saved models, picked, the first the default.
  const [checklist, setChecklist] = createSignal(savedChecklist(existing?.models));
  const [search, setSearch] = createSignal("");
  const [listing, setListing] = createSignal<Listing>(isUsableBaseURL(initialBaseURL) ? { kind: "loading" } : { kind: "needsURL" });
  const [busy, setBusy] = createSignal(false);
  const [spinning, setSpinning] = createSignal(false);
  const [status, setStatus] = createSignal<{ text: string; color: string } | null>(null);
  let closed = false;

  /** The name to save: the one typed, else the known server's name or the host. */
  const savedName = () => name().trim() || suggestedProviderName(baseURL());
  /** While adding, the key's hint follows the server the base URL names. */
  const keyPlaceholder = () => (existing ? undefined : matchingPreset(baseURL())?.keyPlaceholder()) ?? L("Optional for a server on your network");
  const picked = () => checklist().models.filter((model) => checklist().selected.has(model.id));
  const adding = () => addCandidate(search(), checklist().models);
  const rows = () => filterModels(checklist().models, search());
  const isEmpty = () => adding() === undefined && rows().length === 0;
  const canConfirm = () => !busy() && savedName() !== "" && baseURL().trim() !== "" && checklist().selected.size > 0;

  // MARK: The server's models

  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  /** Asks the server for its models once the fields stop changing; a newer request replaces an
   * older one's answer. */
  const loadModels = (delay: number) => {
    // The field that changed was just set, and reads its new value once the update lands.
    flush();
    clearTimeout(timer);
    const current = ++generation;
    const root = baseURL().trim();
    if (!isUsableBaseURL(root)) {
      setListing({ kind: "needsURL" });
      return;
    }
    setListing({ kind: "loading" });
    const request = { name: savedName(), api: api(), baseURL: root, apiKey: key() };
    timer = setTimeout(async () => {
      try {
        const listed = await store.listCustomModels(request);
        if (closed || current !== generation) return;
        // A listing replaces the last one, and a server with none leaves no other server's models
        // behind; picked and typed models stay.
        setChecklist(takeListing(checklist(), listed ?? []));
        if (listed === null || listed.length === 0) {
          setListing({ kind: "unlisted" });
          return;
        }
        setListing({ kind: "listed" });
      } catch (error) {
        if (closed || current !== generation) return;
        setListing({ kind: "failed", message: errorText(error) });
      }
    }, delay);
  };

  // A preset's or a saved provider's server is asked as the sheet opens.
  onSettled(() => {
    if (isUsableBaseURL(initialBaseURL)) queueMicrotask(() => loadModels(0));
  });
  onCleanup(() => {
    closed = true;
    clearTimeout(timer);
  });

  /** A message in place of the list when it has no rows: what the sheet needs, that it is loading,
   * or what the server answered. */
  const overlay = (): string | null => {
    const state = listing();
    switch (state.kind) {
      case "needsURL":
        return L("Enter the base URL to load the server’s models.");
      case "loading":
        return L("Loading models…");
      case "listed":
        return null;
      case "unlisted":
        return L("This server doesn’t list its models. Add model IDs above.");
      case "failed":
        return `${state.message} ${L("You can still add model IDs above.")}`;
    }
  };
  /** With rows on screen, a failure reads in the header instead. */
  const headerNote = () => {
    const state = listing();
    return state.kind === "failed" && !isEmpty() ? state.message : "";
  };

  const add = (id: string) => {
    setChecklist(addModel(checklist(), id));
    setSearch("");
  };

  /** Return in the search field adds the id it holds, or picks the one model it names; with nothing
   * typed it falls through to the sheet's default button. */
  const searchKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.isComposing) return;
    const query = search().trim();
    if (query === "") return;
    event.preventDefault();
    if (busy()) return;
    const id = adding();
    if (id !== undefined) {
      add(id);
      return;
    }
    if (!checklist().selected.has(query)) setChecklist(toggleModel(checklist(), query));
    setSearch("");
  };

  // MARK: Saving

  const begin = (message: string) => {
    setBusy(true);
    setSpinning(true);
    setStatus({ text: message, color: "var(--label-2)" });
  };
  const fail = (error: unknown) => {
    setSpinning(false);
    setBusy(false);
    setStatus({ text: errorText(error), color: "var(--red)" });
  };

  const confirm = async () => {
    if (!canConfirm()) return;
    const saved = savedName();
    begin(L("Checking %@…", saved));
    try {
      const savedKind = await store.saveCustomProvider({ kind, name: saved, api: api(), baseURL: baseURL(), apiKey: key(), models: orderedModelIDs(checklist()) });
      if (closed) return;
      setSpinning(false);
      setStatus({ text: L("%@ connected.", saved), color: "var(--green)" });
      await new Promise((resolve) => setTimeout(resolve, 600));
      if (closed) return;
      props.dismiss();
      props.onSave(savedKind);
    } catch (error) {
      if (!closed) fail(error);
    }
  };

  /** Deletes the provider for the whole account, with no question first, as Disconnect does. */
  const remove = async () => {
    if (!kind) return;
    begin(L("Deleting…"));
    try {
      await store.disconnectProvider(kind);
      if (!closed) props.dismiss();
    } catch (error) {
      if (!closed) fail(error);
    }
  };

  const cancel = () => {
    closed = true;
    props.dismiss();
  };

  return (
    <Sheet
      title={existing ? providerName(existing.kind, store.providers) : preset ? L("Add %@", preset.name) : L("Add Custom Provider")}
      subtitle={L("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Encrypted and shared with your paired Devices.")}
      width={520}
      confirm={kind ? L("Save") : L("Add")}
      confirmDisabled={!canConfirm()}
      onConfirm={() => void confirm()}
      onCancel={cancel}
      leading={
        kind ? (
          <Button kind="destructive" disabled={busy()} onClick={() => void remove()}>
            {L("Delete")}
          </Button>
        ) : undefined
      }
    >
      <div class="custom-provider-form">
        <span class="custom-form-label">{L("Name")}</span>
        <div class="form-control">
          <TextField value={name()} placeholder={suggestedProviderName(baseURL()) || "OpenRouter"} disabled={busy()} label={L("Name")} onInput={setName} />
        </div>
        <span class="custom-form-label">{L("API")}</span>
        <div class="form-control">
          <PopUpButton
            options={customAPIs.map((each) => ({ value: each, label: customAPITitle(each) }))}
            value={api()}
            disabled={busy()}
            label={L("API")}
            onChange={(value) => {
              setAPI(value);
              loadModels(0);
            }}
            class="fill"
          />
        </div>
        <span class="custom-form-label">{L("API base URL")}</span>
        <div class="form-control">
          <TextField
            value={baseURL()}
            placeholder={customBaseURLPlaceholder(api())}
            monospaced
            disabled={busy()}
            label={L("API base URL")}
            onInput={(value) => {
              setBaseURL(value);
              loadModels(500);
            }}
          />
        </div>
        <span />
        <div class="field-note custom-endpoint-note">{customEndpointNote(api(), baseURL())}</div>
        <span class="custom-form-label">{L("API key")}</span>
        <div class="form-control">
          {/* A preset needs its key next; an empty sheet starts at the name. */}
          <APIKeyField
            value={key()}
            placeholder={keyPlaceholder()}
            disabled={busy()}
            autofocus={!existing && initialName !== ""}
            onInput={(value) => {
              setKey(value);
              loadModels(500);
            }}
          />
        </div>
      </div>
      <div class="custom-models">
        <div class="custom-models-header">
          <span class="custom-models-title">{L("Models")}</span>
          <span class="custom-models-note truncate" title={headerNote() || undefined}>
            {headerNote()}
          </span>
          <Show when={listing().kind === "loading"}>
            <Spinner size={12} />
          </Show>
        </div>
        <SearchField value={search()} placeholder={L("Search or add a model ID")} onInput={setSearch} onKeyDown={searchKeyDown} />
        <div class="model-list" role="list" aria-label={L("Models")}>
          <Show when={adding()}>
            {(id) => (
              <button class="model-row model-add" disabled={busy()} onClick={() => add(id())}>
                <Icon name="plus.circle.fill" size={14} strokeWidth={2} class="model-row-mark" />
                <span class="model-row-title truncate">{L("Add “%@”", id())}</span>
              </button>
            )}
          </Show>
          <For each={rows()} keyed={(model) => model.id}>
            {(model) => (
              <label class="model-row" role="listitem">
                <input
                  type="checkbox"
                  class="model-row-mark"
                  checked={checklist().selected.has(model().id)}
                  disabled={busy()}
                  aria-label={modelDisplayName(model())}
                  onChange={() => setChecklist(toggleModel(checklist(), model().id))}
                />
                <span class="model-row-text">
                  <span class="model-row-title truncate">{modelDisplayName(model())}</span>
                  <Show when={modelDisplayName(model()) !== model().id}>
                    <span class="model-row-id truncate">{model().id}</span>
                  </Show>
                </span>
                <Show when={model().contextWindow}>
                  {(tokens) => (
                    <span class="model-row-context" title={L("Context window: %@ tokens", Format.tokens(tokens()))}>
                      {Format.tokens(tokens())}
                    </span>
                  )}
                </Show>
                <Show when={model().images}>
                  <span class="model-row-images" title={L("Sees images")} aria-label={L("Sees images")}>
                    <Icon name="eye" size={13} strokeWidth={1.8} />
                  </span>
                </Show>
              </label>
            )}
          </For>
          <Show when={isEmpty() && overlay()}>
            {(message) => (
              <div class="model-list-message">
                <Show when={listing().kind === "loading"}>
                  <Spinner size={14} />
                </Show>
                <span>{message()}</span>
              </div>
            )}
          </Show>
        </div>
        {/* How many models are picked, and which one bots run without a model of their own. The
            row keeps its height while nothing is picked. */}
        <div class="model-footer">
          <span>{picked().length === 0 ? "" : L("%d selected", picked().length)}</span>
          <span class="model-default" hidden={picked().length === 0}>
            <span>{L("Default")}</span>
            <PopUpButton
              options={picked().map((model) => ({ value: model.id, label: modelDisplayName(model) }))}
              value={checklist().defaultID ?? ""}
              disabled={busy()}
              label={L("Default model")}
              onChange={(id) => setChecklist({ ...checklist(), defaultID: id })}
            />
          </span>
        </div>
      </div>
      <Show when={status()}>
        {(current) => (
          <div class="status-line">
            <Show when={spinning()}>
              <Spinner size={14} />
            </Show>
            <span style={{ color: current().color }}>{current().text}</span>
          </div>
        )}
      </Show>
    </Sheet>
  );
}
