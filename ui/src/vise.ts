// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The vise body field of a setup (decision 0020): how far below the stock's
// bottom the vise's fixed body stands. Parsing kept free of the page so it can
// be tested on its own (ui/tests/vise.test.mjs). The limits are the model's
// (`Vise::check` in crates/model/src/cam.rs): above 0 and at most 500 mm.

/** The most the model accepts for the body depth, in millimetres. */
export const MAX_BODY = 500;

/** What the field means: no body, a depth in millimetres, or why it was refused. */
export type BodyField =
  | { kind: "none" }
  | { kind: "depth"; value: number }
  | { kind: "refused"; reason: "not-a-number" | "not-positive" | "too-deep" };

/** Reads the field's text. Empty (or only spaces) means no body. */
export function parseBody(text: string): BodyField {
  const trimmed = text.trim();
  if (trimmed === "") return { kind: "none" };
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return { kind: "refused", reason: "not-a-number" };
  if (value <= 0) return { kind: "refused", reason: "not-positive" };
  if (value > MAX_BODY) return { kind: "refused", reason: "too-deep" };
  return { kind: "depth", value };
}
