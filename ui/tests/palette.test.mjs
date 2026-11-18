// SPDX-License-Identifier: AGPL-3.0-or-later
// Run with `npm --prefix ui test` (Node's own test runner; no dependency).
import assert from "node:assert/strict";
import { test } from "node:test";
import { firstSentence, matchEntries } from "../src/palette.ts";

const entry = (label, detail = "", keywords = []) => ({ label, detail, keywords });
const entries = [
  entry("Extrude", "Turn a sketch into a solid", ["feature.extrude", "feature.edit_extrude"]),
  entry("Revolve", "Turn a sketch about an axis", ["feature.revolve"]),
  entry("sketch.add_slot", "Add a slot."),
  entry("sketch.add_line", "Add a line."),
  entry("feature.circular_pattern", "Repeat a feature about an axis."),
  entry("Save as…", "Save under a new name"),
];
const labels = (found) => found.map((item) => item.label);

test("an empty query lists everything in the given order", () => {
  assert.deepEqual(labels(matchEntries("", entries)), labels(entries));
  assert.deepEqual(labels(matchEntries("   ", entries)), labels(entries));
});

test("a name matches by prefix, and the closer match comes first", () => {
  assert.equal(labels(matchEntries("ext", entries))[0], "Extrude");
  assert.equal(labels(matchEntries("sketch.add_l", entries))[0], "sketch.add_line");
});

test("a command name finds the button that runs it", () => {
  assert.deepEqual(labels(matchEntries("feature.edit_extrude", entries)), ["Extrude"]);
});

test("a word at the start of a later part of a name matches", () => {
  assert.deepEqual(labels(matchEntries("slot", entries)), ["sketch.add_slot"]);
});

test("the letters in order match, other orders do not", () => {
  assert.deepEqual(labels(matchEntries("fcirc", entries)), ["feature.circular_pattern"]);
  assert.deepEqual(matchEntries("rcf", entries), []);
});

test("every word must fit, in any order", () => {
  assert.deepEqual(labels(matchEntries("add line", entries)), ["sketch.add_line"]);
  assert.deepEqual(matchEntries("add nothing", entries), []);
});

test("the detail finds an entry", () => {
  assert.deepEqual(labels(matchEntries("axis", entries)), ["Revolve", "feature.circular_pattern"]);
  assert.deepEqual(labels(matchEntries("new name", entries)), ["Save as…"]);
});

test("a name ranks above a detail", () => {
  const found = matchEntries("line", [entry("Other", "draws a line"), entry("sketch.add_line")]);
  assert.deepEqual(labels(found), ["sketch.add_line", "Other"]);
});

test("the case of the query does not matter", () => {
  assert.deepEqual(labels(matchEntries("REVOLVE", entries)), ["Revolve"]);
});

test("ties keep the given order and the input is not changed", () => {
  const copy = [...entries];
  assert.deepEqual(labels(matchEntries("add", entries)).slice(0, 2), ["sketch.add_slot", "sketch.add_line"]);
  assert.deepEqual(entries, copy);
});

test("firstSentence stops at the first full stop followed by a space", () => {
  assert.equal(firstSentence("Add a slot. Then more."), "Add a slot.");
  assert.equal(firstSentence("No stop"), "No stop");
  assert.equal(firstSentence("Bevel at 4.5 mm. Done"), "Bevel at 4.5 mm.");
});
