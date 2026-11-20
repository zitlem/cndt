// SPDX-License-Identifier: AGPL-3.0-or-later
// Choosing a side setup's side by clicking a face (decision 0019). Pure: no DOM, no three.js.

/** The sides a side setup can machine from a picked face (model axes: +X right, +Y back, +Z up). */
export type PickedSide = "right" | "left" | "front" | "back";

export type SidePick =
  | { ok: true; side: PickedSide }
  | { ok: false; reason: "up" | "down" | "tilted" | "degenerate" };

/** How far (degrees) a face normal may stray from a side axis and still count as that side. */
export const SIDE_TOLERANCE_DEG = 5;

/**
 * The side a face with this outward normal belongs to. Only the four quarter-turn
 * sides count (decision 0019): a face looking up or down is the top or bottom
 * setup, and a tilted face is refused rather than rounded to the nearest side.
 */
export function sideFromNormal(
  normal: readonly [number, number, number],
  toleranceDeg: number = SIDE_TOLERANCE_DEG,
): SidePick {
  const [x, y, z] = normal;
  const length = Math.hypot(x, y, z);
  if (!Number.isFinite(length) || length < 1e-9) return { ok: false, reason: "degenerate" };
  const axes: { side: PickedSide | "up" | "down"; dot: number }[] = [
    { side: "right", dot: x / length },
    { side: "left", dot: -x / length },
    { side: "back", dot: y / length },
    { side: "front", dot: -y / length },
    { side: "up", dot: z / length },
    { side: "down", dot: -z / length },
  ];
  const best = axes.reduce((a, b) => (b.dot > a.dot ? b : a));
  if (best.dot < Math.cos((toleranceDeg * Math.PI) / 180)) return { ok: false, reason: "tilted" };
  if (best.side === "up") return { ok: false, reason: "up" };
  if (best.side === "down") return { ok: false, reason: "down" };
  return { ok: true, side: best.side };
}
