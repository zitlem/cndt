// SPDX-License-Identifier: AGPL-3.0-or-later
// Naming a picked model edge for `sketch.project_edge` (decision 0033). Pure: no DOM, no three.js.

export type EdgePick =
  | { ok: true; first: string; second: string }
  | { ok: false; reason: "unknown" | "same-face" | "unnamed" | "ambiguous" };

/**
 * The two-face name of an edge. `faces[i]` are the names of face i and `joins[e]` the
 * two faces edge e meets. A face name counts only if it belongs to that face alone;
 * if the same two faces meet at more than one edge the pair does not name this edge,
 * so the pick is refused rather than guessed (decision 0033: a lost or unclear edge says so).
 */
export function edgePairName(
  joins: readonly (readonly [number, number])[],
  faces: readonly (readonly string[])[],
  edge: number,
): EdgePick {
  const join = joins[edge];
  if (!join) return { ok: false, reason: "unknown" };
  const [a, b] = join;
  if (a === b) return { ok: false, reason: "same-face" };
  const unique = (face: number): string | null => {
    const names = faces[face] ?? [];
    return names.find((name) => faces.filter((other) => other.includes(name)).length === 1) ?? null;
  };
  const first = unique(a);
  const second = unique(b);
  if (first === null || second === null) return { ok: false, reason: "unnamed" };
  const same = joins.filter(([x, y]) => (x === a && y === b) || (x === b && y === a)).length;
  if (same !== 1) return { ok: false, reason: "ambiguous" };
  return { ok: true, first, second };
}
