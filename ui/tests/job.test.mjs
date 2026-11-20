// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { progressFraction, progressPercent } from "../src/job.ts";

test("a fraction of the steps done", () => {
  assert.equal(progressFraction([25, 100]), 0.25);
  assert.equal(progressFraction([100, 100]), 1);
});

test("an unknown total gives no fraction", () => {
  assert.equal(progressFraction([0, 0]), null);
  assert.equal(progressFraction([5, 0]), null);
  assert.equal(progressFraction([1, Number.NaN]), null);
});

test("the fraction stays between 0 and 1", () => {
  assert.equal(progressFraction([150, 100]), 1);
  assert.equal(progressFraction([-3, 100]), 0);
});

test("percent rounds down, so 100 means finished", () => {
  assert.equal(progressPercent([199, 200]), 99);
  assert.equal(progressPercent([200, 200]), 100);
  assert.equal(progressPercent([0, 0]), null);
});
