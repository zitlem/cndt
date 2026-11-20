// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { sideFromNormal } from "../src/sidepick.ts";

test("the four quarter-turn sides", () => {
  assert.deepEqual(sideFromNormal([1, 0, 0]), { ok: true, side: "right" });
  assert.deepEqual(sideFromNormal([-1, 0, 0]), { ok: true, side: "left" });
  assert.deepEqual(sideFromNormal([0, 1, 0]), { ok: true, side: "back" });
  assert.deepEqual(sideFromNormal([0, -1, 0]), { ok: true, side: "front" });
});

test("a normal need not be unit length", () => {
  assert.deepEqual(sideFromNormal([0, -37, 0]), { ok: true, side: "front" });
});

test("a face looking up or down is the top or bottom setup, not a side", () => {
  assert.deepEqual(sideFromNormal([0, 0, 1]), { ok: false, reason: "up" });
  assert.deepEqual(sideFromNormal([0, 0, -1]), { ok: false, reason: "down" });
});

test("a tilted face is refused, not rounded to a side", () => {
  const a = (30 * Math.PI) / 180;
  assert.deepEqual(sideFromNormal([Math.cos(a), Math.sin(a), 0]), { ok: false, reason: "tilted" });
  assert.deepEqual(sideFromNormal([1, 0, 1]), { ok: false, reason: "tilted" });
});

test("a face a hair off square still counts, within the tolerance", () => {
  const a = (2 * Math.PI) / 180;
  assert.deepEqual(sideFromNormal([Math.cos(a), Math.sin(a), 0]), { ok: true, side: "right" });
  assert.deepEqual(sideFromNormal([Math.cos(a), Math.sin(a), 0], 1), { ok: false, reason: "tilted" });
});

test("a zero or non-finite normal is refused", () => {
  assert.deepEqual(sideFromNormal([0, 0, 0]), { ok: false, reason: "degenerate" });
  assert.deepEqual(sideFromNormal([NaN, 0, 0]), { ok: false, reason: "degenerate" });
});
