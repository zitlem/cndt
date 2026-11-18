// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { operationRange, visibleMoves } from "../src/scrub.ts";

// Five moves along x: rapid, cut | cut, rapid, cut. Move k runs from x = k to x = k + 1.
const path = {
  coords: Float32Array.from([0, 1, 2, 3, 4].flatMap((k) => [k, 0, 0, k + 1, 0, 0])),
  kinds: Uint8Array.from([0, 1, 1, 0, 1]),
  operations: [
    { name: "Face", first: 0 },
    { name: "Pocket", first: 2 },
  ],
};
const starts = (points) => Array.from(points).filter((_, i) => i % 6 === 0);

test("all operations, all moves", () => {
  const drawn = visibleMoves(path, null, null);
  assert.deepEqual(starts(drawn.rapids), [0, 3]);
  assert.deepEqual(starts(drawn.cuts), [1, 2, 4]);
});

test("one operation shows only its own moves", () => {
  assert.deepEqual(operationRange(path, 0), [0, 2]);
  assert.deepEqual(operationRange(path, 1), [2, 5]);
  const second = visibleMoves(path, 1, null);
  assert.deepEqual(starts(second.rapids), [3]);
  assert.deepEqual(starts(second.cuts), [2, 4]);
});

test("scrubbing stops after the given number of moves", () => {
  const some = visibleMoves(path, 1, 2);
  assert.deepEqual(starts(some.cuts), [2]);
  assert.deepEqual(starts(some.rapids), [3]);
  const none = visibleMoves(path, null, 0);
  assert.equal(none.cuts.length + none.rapids.length, 0);
  assert.deepEqual(starts(visibleMoves(path, null, 1).rapids), [0]);
});

test("a position past the end, a negative one and an unknown operation are safe", () => {
  assert.equal(visibleMoves(path, 0, 99).cuts.length, 6);
  assert.equal(visibleMoves(path, null, -3).cuts.length, 0);
  assert.deepEqual(operationRange(path, 7), [0, 0]);
  assert.equal(visibleMoves(path, 7, null).rapids.length, 0);
});

test("an operation that draws nothing has an empty range", () => {
  const empty = { ...path, operations: [{ name: "A", first: 2 }, { name: "B", first: 2 }] };
  assert.deepEqual(operationRange(empty, 0), [2, 2]);
  assert.deepEqual(operationRange(empty, 1), [2, 5]);
});
