// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from "node:assert/strict";
import { test } from "node:test";
import { pathDistance, splineSamples } from "../src/splinepath.ts";

const near = (a, b, tol = 1e-9) => Math.hypot(a.x - b.x, a.y - b.y) < tol;

test("an open spline passes through its points and ends on the last", () => {
  const pts = [{ x: 0, y: 0 }, { x: 5, y: 4 }, { x: 10, y: 0 }];
  const out = splineSamples(pts, false, 8);
  assert.equal(out.length, 17);
  assert.ok(near(out[0], pts[0]));
  assert.ok(near(out[8], pts[1]));
  assert.ok(near(out[16], pts[2]));
});

test("a closed spline round eight points stays close to the circle and does not repeat its start", () => {
  const pts = Array.from({ length: 8 }, (_, k) => ({
    x: 10 * Math.cos((2 * Math.PI * k) / 8),
    y: 10 * Math.sin((2 * Math.PI * k) / 8),
  }));
  const out = splineSamples(pts, true, 8);
  assert.equal(out.length, 64);
  for (const p of out) assert.ok(Math.abs(Math.hypot(p.x, p.y) - 10) < 0.2, JSON.stringify(p));
  assert.ok(!near(out[63], out[0], 1e-6));
});

test("too few points, or no samples asked for, come back unchanged", () => {
  assert.deepEqual(splineSamples([{ x: 1, y: 2 }], false, 8), [{ x: 1, y: 2 }]);
  const two = [{ x: 0, y: 0 }, { x: 1, y: 1 }];
  assert.deepEqual(splineSamples(two, false, 0), two);
});

test("two points make a straight line", () => {
  const out = splineSamples([{ x: 0, y: 0 }, { x: 10, y: 0 }], false, 4);
  for (const p of out) assert.ok(Math.abs(p.y) < 1e-9);
});

test("distance to a path: on it, off it, and across the closing side", () => {
  const square = [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }, { x: 0, y: 10 }];
  assert.equal(pathDistance({ x: 5, y: 0 }, square, true), 0);
  assert.equal(pathDistance({ x: -3, y: 5 }, square, true), 3);
  // Open: no closing side, so the nearest point is the corner (0, 10).
  assert.equal(pathDistance({ x: -3, y: 5 }, square, false), Math.hypot(3, 5));
  assert.equal(pathDistance({ x: 5, y: -2 }, square, false), 2);
});
