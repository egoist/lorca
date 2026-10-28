// A working character's motion, the desktop app's (desktop/motion.go) on the phone: while its bot
// has a turn running it acts out short clips one after another, drawn as a head in 3D that turns,
// nods, and leans, so the eyes slide across the face and go behind it in a spin or a somersault;
// at rest it stands still. The clips come generated from the desktop app (characters.ts), and
// these worklets run them on the UI thread. `MOTION_SAMPLES` holds the desktop app's poses at a
// few moments, which characterMotion.test.ts checks this copy of the math against.

import { BLINK_LENGTH, CLIPS, type Clip, EXPRESSIONS, FACE_MORPH, HEAD_DEPTH } from "./characters";

export type Pose = {
  yaw: number;
  pitch: number;
  roll: number;
  x: number;
  y: number;
  scale: number;
  /** The eyes' length, width, tilt, and how far they move to the middle of the face. */
  face: [number, number, number, number];
  lids: number;
};

export const REST_POSE: Pose = { yaw: 0, pitch: 0, roll: 0, x: 0, y: 0, scale: 1, face: EXPRESSIONS.rest, lids: 0 };

/// FNV-1a of a key, as the desktop app hashes ids for shapes and clip orders.
export function fnv(key: string): number {
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return hash;
}

function ease(u: number): number {
  "worklet";
  return u * u * (3 - 2 * u);
}

/// A track's value at `t`, eased between the keys around it; a key is [t, a, b, c].
function track(keys: number[][], t: number): number[] {
  "worklet";
  if (t <= keys[0][0]) return keys[0];
  for (let i = 1; i < keys.length; i++) {
    if (t < keys[i][0]) {
      const a = keys[i - 1];
      const b = keys[i];
      const u = ease((t - a[0]) / (b[0] - a[0]));
      return [t, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u, a[3] + (b[3] - a[3]) * u];
    }
  }
  return keys[keys.length - 1];
}

/// The eyes' expression at `t`: the last one turned to, morphing in from the one before.
function face(keys: [number, string][], t: number): [number, number, number, number] {
  "worklet";
  let from = keys[0][1];
  let to = keys[0][1];
  let since = 0;
  for (let i = 0; i < keys.length; i++) {
    if (keys[i][0] > t) break;
    if (i > 0) from = keys[i - 1][1];
    to = keys[i][1];
    since = t - keys[i][0];
  }
  const a = EXPRESSIONS[from];
  const b = EXPRESSIONS[to];
  const u = ease(Math.min(since / FACE_MORPH, 1));
  return [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u, a[3] + (b[3] - a[3]) * u];
}

function clipPose(clip: Clip, t: number): Pose {
  "worklet";
  const turn = track(clip.turn, t);
  const move = track(clip.move, t);
  let lids = 0;
  for (const blink of clip.blinks) {
    const since = t - blink;
    if (since >= 0 && since < BLINK_LENGTH) lids = Math.sin((Math.PI * since) / BLINK_LENGTH);
  }
  return { yaw: turn[1], pitch: turn[2], roll: turn[3], x: move[1], y: move[2], scale: move[3], face: face(clip.face, t), lids };
}

/// The order a bot plays the clips in, shuffled by its seed (xorshift, as the desktop app).
export function clipOrder(seed: number): number[] {
  "worklet";
  const order = CLIPS.map((_, i) => i);
  let state = (seed | 1) >>> 0;
  for (let i = order.length - 1; i > 0; i--) {
    state = (state ^ (state << 13)) >>> 0;
    state = (state ^ (state >>> 17)) >>> 0;
    state = (state ^ (state << 5)) >>> 0;
    const j = state % (i + 1);
    const swap = order[i];
    order[i] = order[j];
    order[j] = swap;
  }
  return order;
}

/// The pose `elapsed` seconds after a character started working: its clips in `order`, round and
/// round.
export function workingPose(elapsed: number, order: number[]): Pose {
  "worklet";
  let loop = 0;
  for (const i of order) loop += CLIPS[i].duration;
  let t = elapsed % loop;
  for (const i of order) {
    if (t < CLIPS[i].duration) return clipPose(CLIPS[i], t);
    t -= CLIPS[i].duration;
  }
  return REST_POSE;
}

/// Where a body point of the box lands for the body in `pose`, in the box: squeezed as a puffy disc
/// is seen turned and tipped, scaled, leaned, and moved. Returns the body's scale on each axis.
export function bodyScale(pose: Pose): [number, number] {
  "worklet";
  const yaw = (pose.yaw * Math.PI) / 180;
  const pitch = (pose.pitch * Math.PI) / 180;
  return [pose.scale * (0.8 + 0.2 * Math.abs(Math.cos(yaw))), pose.scale * (0.6 + 0.4 * Math.abs(Math.cos(pitch)))];
}

/// Where eye `i` of a shape whose eyes sit at `home` shows for a character in `pose`, leaned and
/// moved, in the box, and how much of it faces the viewer: 1 straight on, 0 turned away.
export function eyeAt(home: [number, number][], pose: Pose, i: number): [number, number, number] {
  "worklet";
  const [hx, hy] = home[i];
  // Centered, the pair keeps its spacing about the middle of the face, a little high.
  const cx = hx - (home[0][0] + home[1][0]) / 2;
  const cy = hy - (home[0][1] + home[1][1]) / 2 - 0.1;
  let x = hx + (cx - hx) * pose.face[3];
  let y = hy + (cy - hy) * pose.face[3];
  let z = HEAD_DEPTH * Math.sqrt(Math.max(0.25, 1 - x * x - y * y));
  const yaw = (pose.yaw * Math.PI) / 180;
  const pitch = (pose.pitch * Math.PI) / 180;
  [x, z] = [x * Math.cos(yaw) + z * Math.sin(yaw), -x * Math.sin(yaw) + z * Math.cos(yaw)];
  [y, z] = [y * Math.cos(pitch) - z * Math.sin(pitch), y * Math.sin(pitch) + z * Math.cos(pitch)];
  x *= pose.scale;
  y *= pose.scale;
  const roll = (pose.roll * Math.PI) / 180;
  const facing = Math.min(1, Math.max(0, (z / HEAD_DEPTH - 0.35) / 0.4));
  return [x * Math.cos(roll) - y * Math.sin(roll) + pose.x, x * Math.sin(roll) + y * Math.cos(roll) + pose.y, facing];
}
