// The mockup's fuzzy search: subsequence matches score by closeness.
export function fuzzyScore(hay: string, q: string): number {
  hay = hay.toLowerCase(); q = q.toLowerCase().trim();
  if (!q) return 1;
  let score = 0;
  for (const word of q.split(/\s+/)) {
    if (!word) continue;
    const at = hay.indexOf(word);
    if (at >= 0) { score += 10 + (at === 0 || hay[at - 1] === ' ' ? 5 : 0); continue; }
    let i = 0, run = 0;
    for (const ch of hay) { if (ch === word[i]) { i++; run++; if (i === word.length) break; } }
    if (i < word.length) return 0;
    score += run / word.length;
  }
  return score;
}
