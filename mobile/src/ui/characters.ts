// Made by desktop/characters_test.go from desktop/characters.go; rewrite it with
// LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles . in desktop/, never by hand.

// Bot characters: where their eyes sit and how they move while they work. The bodies' masks
// are in shapeMasks.ts.

export const BOT_SHAPES = ["blob", "pebble", "squircle", "cloud", "drop", "sparkle", "clover", "flower", "heart", "hex", "star", "cushion"] as const;
export type BotShape = (typeof BOT_SHAPES)[number];

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

/// A working character's motion (desktop/motion.go): expressions are the eyes' length, width,
/// tilt, and how far they move to the middle of the face; a clip's turn keys are yaw, pitch,
/// and roll, its move keys x, y, and scale.
export const HEAD_DEPTH = 0.6;
export const FACE_MORPH = 0.28;
export const BLINK_LENGTH = 0.26;

export const EXPRESSIONS: Record<string, [number, number, number, number]> = {
  calm: [0.32, 0.18, 90, 1],
  neutral: [0.46, 0.19, 0, 1],
  rest: [0.43, 0.17, 25, 0],
  surprised: [0.36, 0.27, 0, 1],
};

export type Clip = { name: string; duration: number; turn: number[][]; move: number[][]; face: [number, string][]; blinks: number[] };

export const CLIPS: Clip[] = [
  {
    name: "look",
    duration: 5,
    turn: [[0, 0, 0, 0], [0.7, -28, 6, -6], [1.7, -28, 6, -6], [2.3, 24, -8, 8], [3.3, 24, -8, 8], [3.9, 0, 10, 0], [4.6, 0, 0, 0]],
    move: [[0, 0, 0, 1]],
    face: [[0, "rest"], [0.3, "neutral"], [2.3, "surprised"], [3.3, "neutral"], [4.6, "rest"]],
    blinks: [1.2, 3.6],
  },
  {
    name: "sway",
    duration: 5,
    turn: [[0, 0, 0, 0], [0.5, -6, 10, -30], [1.6, 6, 10, 30], [2.7, -6, 10, -30], [3.8, 6, 10, 30], [4.6, 0, 0, 0]],
    move: [[0, 0, 0, 1], [0.5, -0.06, 0, 1], [1.6, 0.04, 0, 1], [2.7, -0.06, 0, 1], [3.8, 0.04, 0, 1], [4.6, 0, 0, 1]],
    face: [[0, "rest"], [0.2, "surprised"], [4.4, "rest"]],
    blinks: [2.2],
  },
  {
    name: "flip",
    duration: 5,
    turn: [[0, 0, 0, 0], [0.8, -22, -10, -12], [1.4, -22, -10, -12], [2, 18, -12, 9], [2.5, 0, -20, 0], [3.4, 0, -340, 0], [3.8, 0, -360, 0]],
    move: [[0, 0, 0, 1], [2.5, 0, 0.02, 1], [2.9, 0, -0.2, 1], [3.5, 0, -0.12, 1], [4, 0, 0, 1]],
    face: [[0, "rest"], [0.3, "neutral"], [1.9, "surprised"], [4, "neutral"], [4.6, "rest"]],
    blinks: [1.1],
  },
  {
    name: "spin",
    duration: 4.8,
    turn: [[0, 0, 0, 0], [1.3, -16, 2, -30], [2.6, 16, -4, 22], [3.2, 30, 0, 22], [3.9, -270, 0, 6], [4.6, -360, 0, 0]],
    move: [[0, 0, 0, 1], [1.3, 0, 0, 0.72], [2.2, 0, 0, 0.86], [3, 0, 0, 1.03], [4.5, 0, 0, 1]],
    face: [[0, "rest"], [0.4, "neutral"], [2.5, "surprised"], [3.9, "neutral"], [4.5, "rest"]],
    blinks: [0.9],
  },
  {
    name: "glance",
    duration: 5,
    turn: [[0, 0, 0, 0], [0.5, -24, 4, -4], [1.5, -22, -6, -6], [2.2, 24, 8, 10], [2.6, -26, 4, -8], [3.5, 0, 6, 0], [4.2, 24, 6, 8], [4.8, 0, 0, 0]],
    move: [[0, 0, 0, 1], [1.5, 0, 0, 1], [2.2, 0.1, 0.06, 1], [2.6, 0, 0, 1]],
    face: [[0, "rest"], [0.3, "calm"], [4.6, "rest"]],
    blinks: [],
  },
];

/// Poses the desktop app draws for bot-sample: [seconds working, yaw, pitch, roll, x, y, scale, lids, eye length, eye width, eye tilt, eye center].
export const MOTION_SAMPLES: number[][] = [
  [0.5, -6, 10, -30, -0.06, 0, 1, 0, 0.36, 0.27, 0, 1],
  [1.3, 3.809, 10, 19.046, 0.022, 0, 1, 0, 0.36, 0.27, 0, 1],
  [2.75, -5.928, 10, -29.639, -0.059, 0, 1, 0, 0.36, 0.27, 0, 1],
  [3.6, 4.954, 10, 24.771, 0.031, 0, 1, 0, 0.36, 0.27, 0, 1],
  [7.1, 5.44, -2.02, 4.84, 0, 0, 0.855, 0, 0.46, 0.19, 0, 1],
  [9.4, -342.157, 0, 1.19, 0, 0, 1, 0, 0.46, 0.19, 0, 1],
  [12.2, 24, -8, 8, 0, 0, 1, 0, 0.431, 0.213, 0, 1],
  [15.8, -22, -10, -12, 0, 0.007, 1, 0, 0.46, 0.19, 0, 1],
  [19.05, 0, -360, 0, 0, 0, 1, 0, 0.457, 0.193, 0, 1],
  [23.3, 0, 6, 0, 0, 0, 1, 0, 0.32, 0.18, 90, 1],
  [31, -15.731, 1.966, -29.495, 0, 0, 0.725, 0, 0.46, 0.19, 0, 1],
];
