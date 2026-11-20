// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBody } from "../src/vise.ts";

test("an empty field means no body", () => {
  assert.deepEqual(parseBody(""), { kind: "none" });
  assert.deepEqual(parseBody("   "), { kind: "none" });
});

test("a number is a depth", () => {
  assert.deepEqual(parseBody("40"), { kind: "depth", value: 40 });
  assert.deepEqual(parseBody(" 12.5 "), { kind: "depth", value: 12.5 });
  assert.deepEqual(parseBody("500"), { kind: "depth", value: 500 });
});

test("a negative or zero depth is refused", () => {
  assert.deepEqual(parseBody("-5"), { kind: "refused", reason: "not-positive" });
  assert.deepEqual(parseBody("0"), { kind: "refused", reason: "not-positive" });
});

test("text and non-finite numbers are refused", () => {
  assert.deepEqual(parseBody("abc"), { kind: "refused", reason: "not-a-number" });
  assert.deepEqual(parseBody("Infinity"), { kind: "refused", reason: "not-a-number" });
  assert.deepEqual(parseBody("NaN"), { kind: "refused", reason: "not-a-number" });
});

test("more than the model accepts is refused", () => {
  assert.deepEqual(parseBody("500.1"), { kind: "refused", reason: "too-deep" });
});
