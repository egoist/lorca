// The custom provider form's models, shared with the picker it pushes: the rows (saved, added by
// hand, and listed by the server), each picked or not; the default the user chose; and where
// asking the server for its list stands. The form starts it when it opens; both screens change it.

import { create } from "zustand";
import { addModelRow, mergeListedModels, toggleModelRow, type CustomModel, type ModelListing, type ModelRow } from "../core/model";

interface ModelDraft {
  rows: ModelRow[];
  /// The default the Default Model menu chose. While it is not picked, the first picked model is.
  chosenDefault?: string;
  listing: ModelListing;
}

export const useModelDraft = create<ModelDraft>()(() => ({ rows: [], listing: { state: "none" } }));

/// A form opened: its saved rows, the first picked one the default, and the list on its way when
/// the form has a URL to ask.
export function startModelDraft(rows: ModelRow[], asking: boolean) {
  useModelDraft.setState({ rows, chosenDefault: rows.find((row) => row.selected)?.id, listing: { state: asking ? "loading" : "none" } });
}

export function setModelListing(listing: ModelListing) {
  useModelDraft.setState({ listing });
}

/// The server's answer: its models merged into the rows, or that it lists none.
export function takeModelListing(listed: boolean, models: CustomModel[]) {
  useModelDraft.setState((s) => ({ rows: mergeListedModels(s.rows, listed ? models : []), listing: { state: listed && models.length ? "listed" : "unlisted" } }));
}

export function toggleModel(id: string) {
  useModelDraft.setState((s) => ({ rows: toggleModelRow(s.rows, id) }));
}

export function addModel(id: string) {
  useModelDraft.setState((s) => ({ rows: addModelRow(s.rows, id) }));
}

export function chooseDefaultModel(id: string) {
  useModelDraft.setState({ chosenDefault: id });
}
