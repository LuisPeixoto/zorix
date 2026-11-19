// Fuzzy matching for the command palette: letters in order, with a bonus for starts of words and runs.

/** A score for how well `query` matches `text` (higher is better), or `null` when it does not match. */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  // A whole-word or prefix hit beats a scattered one.
  const at = t.indexOf(q);
  if (at === 0) return 1000 - t.length;
  if (at > 0) return (/[\s\-_./:]/.test(t[at - 1]) ? 800 : 600) - at - t.length / 10;
  let score = 0;
  let from = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const found = t.indexOf(ch, from);
    if (found < 0) return null;
    run = found === from ? run + 1 : 0;
    const boundary = found === 0 || /[\s\-_./:]/.test(t[found - 1]);
    score += 10 + run * 8 + (boundary ? 12 : 0) - Math.min(found - from, 12);
    from = found + 1;
  }
  return score;
}

/** Keep the items that match, best first. Ties keep their order. */
export function fuzzyFilter<T>(items: T[], query: string, text: (item: T) => string): T[] {
  if (!query.trim()) return items;
  return items
    .map((item, index) => ({ item, index, score: fuzzyScore(query, text(item)) }))
    .filter((entry): entry is { item: T; index: number; score: number } => entry.score !== null)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.item);
}
