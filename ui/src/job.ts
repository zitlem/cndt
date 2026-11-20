// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Progress of a long job (simulation, verification; decision 0035) as a
// fraction and a percentage. Kept free of the page so it can be tested on its
// own (ui/tests/job.test.mjs).

/** Steps done, steps in all, as `job_progress` returns them. A total of 0 means "not known yet". */
export type Progress = [done: number, total: number];

/** How far along, from 0 to 1; `null` while the total is not known. */
export function progressFraction([done, total]: Progress): number | null {
  if (!Number.isFinite(done) || !Number.isFinite(total) || total <= 0) return null;
  return Math.min(1, Math.max(0, done / total));
}

/** A whole percentage for display, or `null` while the total is not known. */
export function progressPercent(progress: Progress): number | null {
  const fraction = progressFraction(progress);
  return fraction === null ? null : Math.floor(fraction * 100);
}
