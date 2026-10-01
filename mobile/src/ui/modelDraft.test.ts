import { describe, expect, test } from "bun:test";
import { defaultModelId, selectedModelIds } from "../core/model";
import { addModel, chooseDefaultModel, startModelDraft, takeModelListing, toggleModel, useModelDraft } from "./modelDraft";

const defaultId = () => defaultModelId(useModelDraft.getState().rows, useModelDraft.getState().chosenDefault);
const listing = (count: number) => Array.from({ length: count }, (_, index) => ({ id: `m${index}` }));

describe("the custom provider form's default model", () => {
  test("stays on the first model picked while others are picked or added", () => {
    startModelDraft([], true);
    takeModelListing(true, listing(12));
    expect(defaultId()).toBeUndefined();
    toggleModel("m5");
    expect(defaultId()).toBe("m5");
    // Picked higher in the list, and an id added at the top: the default stays.
    toggleModel("m1");
    addModel("my-finetune");
    expect(defaultId()).toBe("m5");
    expect(selectedModelIds(useModelDraft.getState().rows, useModelDraft.getState().chosenDefault)).toEqual(["m5", "my-finetune", "m1"]);
  });

  test("moves to the first picked model left when it is unpicked, and follows the menu", () => {
    startModelDraft([], true);
    takeModelListing(true, listing(12));
    toggleModel("m5");
    toggleModel("m1");
    toggleModel("m5");
    expect(defaultId()).toBe("m1");
    toggleModel("m3");
    chooseDefaultModel("m3");
    toggleModel("m0");
    expect(defaultId()).toBe("m3");
  });
});
