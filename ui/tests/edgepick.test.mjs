// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { edgePairName } from "../src/edgepick.ts";

const faces = [["top"], ["front"], ["side", "side-1"], ["shared"], ["shared"]];

test("an edge between two uniquely named faces is named by them", () => {
  assert.deepEqual(edgePairName([[0, 1]], faces, 0), { ok: true, first: "top", second: "front" });
});

test("the first name that is unique to the face is used", () => {
  assert.deepEqual(edgePairName([[2, 0]], faces, 0), { ok: true, first: "side", second: "top" });
});

test("an edge that does not exist is refused", () => {
  assert.deepEqual(edgePairName([[0, 1]], faces, 5), { ok: false, reason: "unknown" });
  assert.deepEqual(edgePairName([], faces, 0), { ok: false, reason: "unknown" });
});

test("a face meeting itself cannot name an edge", () => {
  assert.deepEqual(edgePairName([[1, 1]], faces, 0), { ok: false, reason: "same-face" });
});

test("a face with no name of its own is refused", () => {
  // faces 3 and 4 both carry only "shared"
  assert.deepEqual(edgePairName([[0, 3]], faces, 0), { ok: false, reason: "unnamed" });
  assert.deepEqual(edgePairName([[0, 9]], faces, 0), { ok: false, reason: "unnamed" });
});

test("two faces that meet at more than one edge are refused, either order", () => {
  const joins = [
    [0, 1],
    [1, 0],
    [0, 2],
  ];
  assert.deepEqual(edgePairName(joins, faces, 0), { ok: false, reason: "ambiguous" });
  assert.deepEqual(edgePairName(joins, faces, 1), { ok: false, reason: "ambiguous" });
  assert.deepEqual(edgePairName(joins, faces, 2), { ok: true, first: "top", second: "side" });
});
