import { expect, test } from "bun:test";
import { CLIPS, MOTION_SAMPLES } from "./characters";
import { clipOrder, fnv, workingPose } from "./characterMotion";

test("the phone draws the desktop app's working poses", () => {
  const order = clipOrder(fnv("bot-sample"));
  expect([...order].sort()).toEqual(CLIPS.map((_, i) => i));
  for (const [t, yaw, pitch, roll, x, y, scale, lids, ...face] of MOTION_SAMPLES) {
    const pose = workingPose(t, order);
    const got = [pose.yaw, pose.pitch, pose.roll, pose.x, pose.y, pose.scale, pose.lids, ...pose.face];
    [yaw, pitch, roll, x, y, scale, lids, ...face].forEach((want, i) => expect(Math.abs(got[i] - want)).toBeLessThan(0.002));
  }
});
