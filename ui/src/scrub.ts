// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Scrubbing a toolpath preview: which of its moves to draw when the person
// picks one operation, or drags to a move part-way along. Kept free of the
// page so it can be tested on its own (ui/tests/scrub.test.mjs).

/** A setup's toolpath as straight moves in model coordinates, grouped by operation. */
export interface Toolpath {
  /** Six numbers per move: x, y, z from, then x, y, z to. */
  coords: Float32Array;
  /** One byte per move: 0 for a rapid, 1 for a cut. */
  kinds: Uint8Array;
  /** Where each operation's moves start, in program order. */
  operations: { name: string; first: number }[];
}

/** What to hand the viewer: the same moves split into rapids and cuts. */
export interface DrawnPath {
  rapids: Float32Array;
  cuts: Float32Array;
}

/** The moves of operation `operation` (all of them when `null`) as `[first, end)`. */
export function operationRange(path: Toolpath, operation: number | null): [number, number] {
  const total = path.kinds.length;
  if (operation === null) return [0, total];
  const start = path.operations[operation]?.first;
  if (start === undefined) return [0, 0];
  const end = path.operations[operation + 1]?.first ?? total;
  return [Math.min(start, total), Math.min(Math.max(end, start), total)];
}

/**
 * The moves to draw: those of `operation` (all when `null`), only the first
 * `upTo` of them (all when `null`). `upTo` is clamped to what exists.
 */
export function visibleMoves(path: Toolpath, operation: number | null, upTo: number | null): DrawnPath {
  const [first, end] = operationRange(path, operation);
  const last = upTo === null ? end : Math.min(end, first + Math.max(0, Math.floor(upTo)));
  const rapids: number[] = [];
  const cuts: number[] = [];
  for (let move = first; move < last; move++) {
    const out = path.kinds[move] === 0 ? rapids : cuts;
    for (let k = 0; k < 6; k++) out.push(path.coords[move * 6 + k] ?? 0);
  }
  return { rapids: new Float32Array(rapids), cuts: new Float32Array(cuts) };
}
