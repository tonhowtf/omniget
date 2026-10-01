export function smartClean(text: string, cols: number): string {
  const lines = text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .filter((l) => !/^[\s╭╮╰╯┌┐└┘├┤┬┴┼─━═│┃║]+$/.test(l) || l.trim() === '')
    .map((l) => l.replace(/^\s*[│┃║]\s?/, '').replace(/\s?[│┃║]\s*$/, '').replace(/\s+$/, ''));
  const out: string[] = [];
  let prevWidth = 0;
  for (const raw of lines) {
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

export function tokenAt(cells: string[], col: number): string {
  if (col < 0 || col >= cells.length || !cells[col].trim()) return '';
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
  while (tok.startsWith('(') && count(tok, '(') > count(tok, ')')) tok = tok.slice(1);
  while (tok.endsWith(')') && count(tok, ')') > count(tok, '(')) tok = tok.slice(0, -1);
  return tok.replace(/:$/, '').replace(/:(\d+)(:\d+)?$/, '');
}

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
  if (p.startsWith('~/')) return null;
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
