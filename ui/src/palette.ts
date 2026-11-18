// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The command palette's matching, kept free of the page so it can be tested
// on its own (ui/tests/palette.test.mjs). main.ts builds the entries from the
// engine's command list and the page's buttons and shows the result.

export interface PaletteEntry {
  /** What the entry is called: a button's label, or a command's name. */
  label: string;
  /** One line of explanation, searched after the label. */
  detail: string;
  /** Other names that find the entry, such as the engine commands a button runs. */
  keywords: readonly string[];
}

/** Rank of how well one word of the query fits a text; lower is better, null means no fit. */
function fit(word: string, text: string): number | null {
  const lower = text.toLowerCase();
  if (lower === word) return 0;
  if (lower.startsWith(word)) return 1;
  const at = lower.indexOf(word);
  if (at > 0) {
    // The start of a later piece ("add_slot" for "slot") beats the middle of a word.
    return /[a-z0-9]/.test(lower.charAt(at - 1)) ? 3 : 2;
  }
  // The letters in order, not together: "fcirc" finds "feature.circular_pattern".
  let next = 0;
  for (const letter of lower) {
    if (letter === word.charAt(next)) next += 1;
    if (next === word.length) return 4;
  }
  return null;
}

/** The best rank of a word over the label and keywords, then the detail, which counts for less. */
function wordRank(word: string, entry: PaletteEntry): number | null {
  let best: number | null = null;
  const consider = (rank: number | null, penalty: number) => {
    if (rank === null) return;
    const total = rank + penalty;
    if (best === null || total < best) best = total;
  };
  consider(fit(word, entry.label), 0);
  for (const keyword of entry.keywords) consider(fit(word, keyword), 0);
  // A detail only counts when the word appears whole in it, not as scattered letters.
  if (entry.detail.toLowerCase().includes(word)) consider(2, 4);
  return best;
}

/**
 * The entries that fit the query, best first. Every word of the query (split
 * on spaces) must fit; ties keep the order the entries were given in, so the
 * result is the same every time. An empty query returns every entry as given.
 */
export function matchEntries<T extends PaletteEntry>(query: string, entries: readonly T[]): T[] {
  const words = query
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word.length > 0);
  if (words.length === 0) return [...entries];
  const ranked: { entry: T; rank: number; index: number }[] = [];
  entries.forEach((entry, index) => {
    let rank = 0;
    for (const word of words) {
      const one = wordRank(word, entry);
      if (one === null) return;
      rank += one;
    }
    ranked.push({ entry, rank, index });
  });
  ranked.sort((a, b) => a.rank - b.rank || a.index - b.index);
  return ranked.map((item) => item.entry);
}

/** The first sentence of a command's description, for the one-line detail. */
export function firstSentence(description: string): string {
  const end = description.search(/\.(\s|$)/);
  return end < 0 ? description : description.slice(0, end + 1);
}
