function fold(s: string): string {
  let out = '';
  for (const ch of s) {
    const base = ch.normalize('NFD').replace(/[̀-ͯ]/g, '');
    const low = (base || ch).toLowerCase();
    out += low.length === ch.length ? low : low.padEnd(ch.length, ' ').slice(0, ch.length);
  }
  return out;
}

function isBoundary(text: string, i: number): boolean {
  if (i === 0) return true;
  const p = text[i - 1];
  return /[\s\-_./\\:@#([]/.test(p) || (text[i] !== text[i].toLowerCase() && p === p.toLowerCase());
}

function matchWord(word: string, folded: string, original: string, loose = true): { score: number; idx: number[] } | null {
  if (!word) return { score: 0, idx: [] };
  const sub = folded.indexOf(word);
  if (sub >= 0) {
    const idx = Array.from({ length: word.length }, (_, k) => sub + k);
    let score = 10 + word.length * 3;
    if (isBoundary(original, sub)) score += 8;
    if (sub === 0) score += 6;
    return { score, idx };
  }
  if (!loose) return null;
  const idx: number[] = [];
  let score = 0;
  let from = 0;
  let prev = -2;
  for (const ch of word) {
    const i = folded.indexOf(ch, from);
    if (i < 0) return null;
    idx.push(i);
    score += 1;
    if (i === prev + 1) score += 3;
    if (isBoundary(original, i)) score += 4;
    score -= Math.min(3, (i - from) * 0.05);
    prev = i;
    from = i + 1;
  }
  if (idx[idx.length - 1] - idx[0] + 1 > Math.max(8, word.length * 4)) return null;
  return { score, idx };
}

export interface Field {
  text: string;
  weight: number;
  loose?: boolean;
}

interface FuzzyResult {
  score: number;
  nameIdx: number[];
}

export function splitQuery(q: string): string[] {
  return fold(q).split(/\s+/).filter(Boolean);
}

export function fuzzy(words: string[], fields: Field[]): FuzzyResult | null {
  if (!words.length) return { score: 0, nameIdx: [] };
  const folded = fields.map((f) => fold(f.text));
  let total = 0;
  const nameIdx = new Set<number>();
  for (const w of words) {
    let best = -Infinity;
    let bestIdx: number[] | null = null;
    let bestField = -1;
    for (let k = 0; k < fields.length; k++) {
      if (!fields[k].text) continue;
      const m = matchWord(w, folded[k], fields[k].text, fields[k].loose ?? true);
      if (!m) continue;
      const s = m.score * fields[k].weight;
      if (s > best) {
        best = s;
        bestIdx = m.idx;
        bestField = k;
      }
    }
    if (bestIdx === null) return null;
    total += best;
    if (bestField === 0) for (const i of bestIdx) nameIdx.add(i);
  }
  return { score: total, nameIdx: Array.from(nameIdx).sort((a, b) => a - b) };
}

export function highlightRuns(text: string, idx: number[]): Array<{ s: string; hit: boolean }> {
  if (!idx.length) return [{ s: text, hit: false }];
  const set = new Set(idx);
  const out: Array<{ s: string; hit: boolean }> = [];
  for (let i = 0; i < text.length; i++) {
    const hit = set.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.s += text[i];
    else out.push({ s: text[i], hit });
  }
  return out;
}
