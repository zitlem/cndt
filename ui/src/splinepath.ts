// SPDX-License-Identifier: AGPL-3.0-or-later
// Points along a fit-point spline, for drawing and picking in the sketcher. Pure: no DOM.
// The same centripetal Catmull-Rom curve the sketch core uses (crates/sketch/src/spline.rs),
// so the picture and the regions the core finds agree. The kernel's own curve is a
// cubic B-spline through the same points and stays within a drawing pixel of this one.

export interface Pt {
  x: number;
  y: number;
}

const gap = (a: Pt, b: Pt): number => Math.hypot(a.x - b.x, a.y - b.y);

function centripetal(p0: Pt, p1: Pt, p2: Pt, p3: Pt, t: number): Pt {
  const knot = (a: Pt, b: Pt): number => Math.max(Math.sqrt(gap(a, b)), 1e-9);
  const t1 = knot(p0, p1);
  const t2 = t1 + knot(p1, p2);
  const t3 = t2 + knot(p2, p3);
  const u = t1 + (t2 - t1) * t;
  const lerp = (a: Pt, b: Pt, ta: number, tb: number): Pt => {
    const w = (u - ta) / (tb - ta);
    return { x: a.x + (b.x - a.x) * w, y: a.y + (b.y - a.y) * w };
  };
  const a1 = lerp(p0, p1, 0, t1);
  const a2 = lerp(p1, p2, t1, t2);
  const a3 = lerp(p2, p3, t2, t3);
  return lerp(lerp(a1, a2, 0, t2), lerp(a2, a3, t1, t3), t1, t2);
}

/**
 * Points along the spline through `points`, `perSpan` for each stretch between two
 * neighbours, from the first point on. An open spline ends on the last point; a closed
 * one returns to the first and stops short of repeating it. Fewer than two points, or
 * no samples asked for, come back unchanged.
 */
export function splineSamples(points: readonly Pt[], closed: boolean, perSpan: number): Pt[] {
  const n = points.length;
  if (n < 2 || perSpan < 1) return [...points];
  const first = points[0] as Pt;
  const second = points[1] as Pt;
  const last = points[n - 1] as Pt;
  const beforeLast = points[n - 2] as Pt;
  const reflect = (a: Pt, b: Pt): Pt => ({ x: 2 * a.x - b.x, y: 2 * a.y - b.y });
  const at = (i: number): Pt => {
    if (closed) return points[((i % n) + n) % n] as Pt;
    if (i < 0) return reflect(first, second);
    if (i >= n) return reflect(last, beforeLast);
    return points[i] as Pt;
  };
  const spans = closed ? n : n - 1;
  const out: Pt[] = [];
  for (let s = 0; s < spans; s++) {
    for (let k = 0; k < perSpan; k++) {
      out.push(centripetal(at(s - 1), at(s), at(s + 1), at(s + 2), k / perSpan));
    }
  }
  if (!closed) out.push(last);
  return out;
}

/** Distance from `p` to the polyline `path` (closed back on its start if `closed`). */
export function pathDistance(p: Pt, path: readonly Pt[], closed: boolean): number {
  let best = Infinity;
  const count = closed ? path.length : path.length - 1;
  for (let i = 0; i < count; i++) {
    const a = path[i] as Pt;
    const b = path[(i + 1) % path.length] as Pt;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const len2 = dx * dx + dy * dy;
    const t = len2 > 0 ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2)) : 0;
    best = Math.min(best, gap(p, { x: a.x + t * dx, y: a.y + t * dy }));
  }
  return best;
}
