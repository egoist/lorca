// Made by desktop/characters_test.go from desktop/characters.go; rewrite it with
// LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles . in desktop/, never by hand.

// Bot characters' bodies as white masks to tint.

import type { BotShape } from "./characters";

export const SHAPE_MASKS: Record<BotShape, number> = {
  blob: require("../../assets/shapes/blob.png"),
  pebble: require("../../assets/shapes/pebble.png"),
  squircle: require("../../assets/shapes/squircle.png"),
  cloud: require("../../assets/shapes/cloud.png"),
  drop: require("../../assets/shapes/drop.png"),
  sparkle: require("../../assets/shapes/sparkle.png"),
  clover: require("../../assets/shapes/clover.png"),
  flower: require("../../assets/shapes/flower.png"),
  heart: require("../../assets/shapes/heart.png"),
  hex: require("../../assets/shapes/hex.png"),
  star: require("../../assets/shapes/star.png"),
  cushion: require("../../assets/shapes/cushion.png"),
};
