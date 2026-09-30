// Pure text helpers of the terminal input layer (no runes, testable):
// Smart Copy, the token under a ⌘-click and path resolution.

/**
 * Smart Copy: the selection as clean paragraphs. Box borders and prompt
 * glyphs of TUIs go, trailing blanks go, and lines that the terminal broke at
 * the right margin are joined back (a line that fills the width continues on
 * the next one). Blank lines, lists and indented code keep their breaks.
 */
export function smartClean(text: string, cols: number): string {
  const lines = text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    // Frame-only lines (╭──╮, ────) go; side borders (│ text │) are cut off.
    .filter((l) => !/^[\s╭╮╰╯┌┐└┘├┤┬┴┼─━═│┃║]+$/.test(l) || l.trim() === '')
    .map((l) => l.replace(/^\s*[│┃║]\s?/, '').replace(/\s?[│┃║]\s*$/, '').replace(/\s+$/, ''));
  const out: string[] = [];
  let prevWidth = 0;
  for (const raw of lines) {
    // Prompt and bullet glyphs of agent TUIs (❯ ⏺ ⎿).
    const line = raw.replace(/^\s*[❯›⏺⎿]\s?/, '');
    const prev = out.length ? out[out.length - 1] : null;
    const continues =
      prev !== null &&
      prev.trim() !== '' &&
      line.trim() !== '' &&
      !/^\s*([-*+•]|\d+[.)])\s/.test(line) &&
      !/^\s{4,}/.test(line) &&
      (prevWidth >= cols - 2 || (/[^.!?:;]$/.test(prev.trim()) && /^\s*[a-z(]/.test(line)));
    if (continues) {
      out[out.length - 1] = prev.replace(/\s+$/, '') + (/[-/]$/.test(prev) ? '' : ' ') + line.trim();
    } else {
      out.push(line.trim() === '' ? '' : line.replace(/^\s{1,3}(?=\S)/, ''));
    }
    prevWidth = raw.length;
  }
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

/**
 * The URL or path under a cell (⌘-click): quoted text when the click is
 * inside quotes, else the run of non-blank cells, trimmed of punctuation
 * that usually ends a sentence. Parentheses inside a path are kept
 * (`src/(app)/page.tsx`).
 */
export function tokenAt(cells: string[], col: number): string {
  if (col < 0 || col >= cells.length || !cells[col].trim()) return '';
  // Inside quotes: the quoted text (paths with spaces).
  for (const q of ['"', "'", '`']) {
    const left = cells.slice(0, col).lastIndexOf(q);
    const right = cells.indexOf(q, col + 1);
    if (left !== -1 && right !== -1 && cells[col] !== q) {
      const inner = cells.slice(left + 1, right).join('');
      if (inner.trim() && !inner.includes(q)) return inner;
    }
  }
  let a = col;
  let b = col;
  while (a > 0 && cells[a - 1].trim()) a--;
  while (b < cells.length - 1 && cells[b + 1].trim()) b++;
  let tok = cells.slice(a, b + 1).join('');
  const count = (t: string, c: string) => t.split(c).length - 1;
  tok = tok.replace(/^[<\[{"'`]+/, '');
  tok = tok.replace(/[.,;!?>\]}"'`]+$/, '');
  // Unbalanced parentheses at the ends are sentence punctuation.
  while (tok.startsWith('(') && count(tok, '(') > count(tok, ')')) tok = tok.slice(1);
  while (tok.endsWith(')') && count(tok, ')') > count(tok, '(')) tok = tok.slice(0, -1);
  // file:line:col suffixes (compiler output) are not part of the path.
  return tok.replace(/:$/, '').replace(/:(\d+)(:\d+)?$/, '');
}

/** Absolute path for `p` relative to the shell's directory (OSC 7), or null. */
export function resolvePath(p: string, cwd: string): string | null {
  if (!p) return null;
  let base = cwd;
  if (base.startsWith('file://')) {
    try {
      base = decodeURIComponent(new URL(base).pathname);
    } catch {
      base = base.replace(/^file:\/\/[^/]*/, '');
    }
  }
  if (p.startsWith('~/')) return null; // expanded by Rust when opened
  if (p.startsWith('/')) return p;
  if (/^[A-Za-z]:[\\/]/.test(p)) return p;
  if (!base) return null;
  const parts = (base.replace(/\/+$/, '') + '/' + p).split('/');
  const out: string[] = [];
  for (const s of parts) {
    if (s === '..') out.pop();
    else if (s !== '.' && s !== '') out.push(s);
  }
  return '/' + out.join('/');
}
