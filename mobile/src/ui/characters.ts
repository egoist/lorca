// Made by desktop/characters_test.go from desktop/characters.go; rewrite it with
// LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles . in desktop/, never by hand.

// Bot characters' bodies as white masks to tint, and where their eyes sit.

export const BOT_SHAPES = ["blob", "pebble", "squircle", "cloud", "drop", "sparkle", "clover", "flower", "heart", "hex", "star", "cushion"] as const;
export type BotShape = (typeof BOT_SHAPES)[number];

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

/// Each eye's middle in a box from -1 to 1, y down. An eye is EYE_WIDTH wide and EYE_LENGTH
/// long in that box, with round ends, its top leaning EYE_TILT degrees to the left.
export const EYE_WIDTH = 0.17;
export const EYE_LENGTH = 0.43;
export const EYE_TILT = 25;

export const SHAPE_EYES: Record<BotShape, [number, number][]> = {
  blob: [[0.04, -0.2], [0.44, -0.3]],
  pebble: [[0.04, -0.2], [0.44, -0.3]],
  squircle: [[0.04, -0.2], [0.44, -0.3]],
  cloud: [[0.02, -0.16], [0.42, -0.26]],
  drop: [[0.06, -0.14], [0.46, -0.24]],
  sparkle: [[-0.18, 0.04], [0.22, -0.06]],
  clover: [[-0.02, -0.1], [0.38, -0.2]],
  flower: [[-0.04, -0.1], [0.36, -0.2]],
  heart: [[-0.08, 0], [0.32, -0.1]],
  hex: [[0.04, -0.2], [0.44, -0.3]],
  star: [[-0.2, 0.04], [0.2, -0.06]],
  cushion: [[0.04, -0.2], [0.44, -0.3]],
};
