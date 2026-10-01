export interface PathMatch {
  start: number;
  end: number;
  path: string;
  line?: number;
}

const CANDIDATE =
  /(^|[\s(["'`<>,;])((?:~\/|\.{1,2}\/|\/|[A-Za-z]:\\)[^\s"'`<>()[\]{}|,;]*|[\w@.+-]+\/(?:[\w@.+-]+\/)*[\w@+-][\w@.+-]*\.[A-Za-z0-9]{1,8})(?::(\d+)(?::\d+)?)?/g;

const TRAILING = /[.,:;!?)\]'"`]+$/;

export function findPaths(text: string): PathMatch[] {
  const out: PathMatch[] = [];
  if (!text || !/[/\\]/.test(text)) return out;
  CANDIDATE.lastIndex = 0;
  for (let m = CANDIDATE.exec(text); m; m = CANDIDATE.exec(text)) {
    let raw = m[2];
    const start = m.index + m[1].length;
    const before = text.slice(Math.max(0, start - 12), start);
    if (/[a-z][a-z0-9+.-]*:\/?$/i.test(before) || raw.startsWith('//')) continue;
    let line: number | undefined;
    let end = start + raw.length;
    if (m[3]) {
      line = Number(m[3]);
      end = m.index + m[0].length;
    } else {
      const trimmed = raw.replace(TRAILING, '');
      end -= raw.length - trimmed.length;
      raw = trimmed;
    }
    if (raw.length < 2 || raw === '/' || raw === '~/' || /^\/+$/.test(raw)) continue;
    if (raw.startsWith('/') && !/\/[^/]+\//.test(raw) && !/\.[A-Za-z0-9]{1,8}$/.test(raw)) continue;
    out.push({ start, end, path: raw, line });
  }
  return out;
}
