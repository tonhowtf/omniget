// Small syntax highlighter for fenced code in notes and chat (Maestri colours
// fenced blocks by the info string). A table-driven tokenizer per language
// family: comments, strings, numbers, keywords, types, function calls and
// punctuation. No dependency; output is a list of tokens the caller turns
// into DOM (never HTML strings), so nothing here can inject markup.

export type TokenKind = 'plain' | 'comment' | 'string' | 'number' | 'keyword' | 'type' | 'fn' | 'punct' | 'tag' | 'attr' | 'var';

export interface Token {
  kind: TokenKind;
  text: string;
}

interface Lang {
  keywords: Set<string>;
  types?: Set<string>;
  literals?: Set<string>;
  line?: string[];
  block?: [string, string][];
  strings: string[];
  /** Triple-quoted strings (python). */
  triple?: boolean;
  /** `$name` variables (bash, php). */
  dollarVars?: boolean;
  /** Rust lifetimes / char literals need care: 'a is not a string start. */
  rustChars?: boolean;
  /** Everything is case-insensitive keywords (sql). */
  ci?: boolean;
}

const words = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const JS: Lang = {
  keywords: words(
    'as async await break case catch class const continue debugger default delete do else enum export extends finally for from function get if implements import in instanceof interface let new of package private protected public readonly return satisfies set static super switch this throw try type typeof var void while with yield keyof declare namespace abstract',
  ),
  types: words('string number boolean any unknown never object symbol bigint Promise Array Record Map Set Partial Readonly'),
  literals: words('true false null undefined NaN Infinity'),
  line: ['//'],
  block: [['/*', '*/']],
  strings: ['"', "'", '`'],
};

const RUST: Lang = {
  keywords: words(
    'as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while macro_rules',
  ),
  types: words('i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str String Vec Option Result Box Rc Arc HashMap HashSet'),
  literals: words('true false None Some Ok Err'),
  line: ['//'],
  block: [['/*', '*/']],
  strings: ['"'],
  rustChars: true,
};

const PY: Lang = {
  keywords: words(
    'and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case',
  ),
  types: words('int float str bool list dict set tuple bytes object type'),
  literals: words('True False None self'),
  line: ['#'],
  strings: ['"', "'"],
  triple: true,
};

const SH: Lang = {
  keywords: words('if then else elif fi for while until do done case esac in function return local export source alias unset readonly shift exit break continue select time'),
  types: words('echo cd ls cat grep sed awk git npm pnpm cargo sudo rm mkdir cp mv curl printf read test'),
  literals: words('true false'),
  line: ['#'],
  strings: ['"', "'"],
  dollarVars: true,
};

const GO: Lang = {
  keywords: words('break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var'),
  types: words('int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 float32 float64 string bool byte rune error any'),
  literals: words('true false nil iota'),
  line: ['//'],
  block: [['/*', '*/']],
  strings: ['"', '`', "'"],
};

const CLIKE: Lang = {
  keywords: words(
    'abstract auto break case catch class const continue default delete do else enum extends extern final finally for friend fun func goto guard if implements import in inline interface internal let namespace new object operator override package private protected public return sealed static struct super switch template this throw throws try typedef typename union using val var virtual void volatile when where while',
  ),
  types: words('int long short char float double bool boolean byte string String Int Long Double Float Boolean Unit Any size_t'),
  literals: words('true false null nil nullptr NULL'),
  line: ['//'],
  block: [['/*', '*/']],
  strings: ['"', "'"],
};

const CSS: Lang = {
  keywords: words('important media import supports keyframes font-face from to and not only screen'),
  line: [],
  block: [['/*', '*/']],
  strings: ['"', "'"],
};

const JSON_L: Lang = {
  keywords: new Set(),
  literals: words('true false null'),
  strings: ['"'],
};

const SQL: Lang = {
  keywords: words(
    'select from where and or not insert into values update set delete create table index view drop alter add column primary key foreign references join left right inner outer full on as group by order having limit offset union all distinct case when then else end is null like in between exists with returning default begin commit rollback',
  ),
  types: words('integer int text varchar char boolean real float double date timestamp blob serial bigint numeric'),
  literals: words('true false null'),
  line: ['--'],
  block: [['/*', '*/']],
  strings: ["'", '"'],
  ci: true,
};

const YAML: Lang = {
  keywords: new Set(),
  literals: words('true false null yes no on off'),
  line: ['#'],
  strings: ['"', "'"],
};

const LANGS: Record<string, Lang | 'markup'> = {
  js: JS, javascript: JS, jsx: JS, mjs: JS, cjs: JS, ts: JS, typescript: JS, tsx: JS,
  json: JSON_L, jsonc: JS, json5: JS,
  rust: RUST, rs: RUST,
  python: PY, py: PY,
  bash: SH, sh: SH, shell: SH, zsh: SH, console: SH, fish: SH,
  go: GO, golang: GO,
  c: CLIKE, h: CLIKE, cpp: CLIKE, 'c++': CLIKE, cc: CLIKE, cs: CLIKE, csharp: CLIKE, java: CLIKE, kotlin: CLIKE, kt: CLIKE, swift: CLIKE, scala: CLIKE, dart: CLIKE, php: { ...CLIKE, dollarVars: true },
  css: CSS, scss: CSS, less: CSS,
  sql: SQL, sqlite: SQL, postgres: SQL,
  yaml: YAML, yml: YAML, toml: YAML, ini: YAML,
  html: 'markup', xml: 'markup', svg: 'markup', svelte: 'markup', vue: 'markup',
};

/** Language of a `language-xxx` class or info string, or null when unknown. */
export function langOf(info: string | null | undefined): string | null {
  const l = (info ?? '').trim().toLowerCase().split(/\s+/)[0] ?? '';
  return l && l in LANGS ? l : null;
}

function push(out: Token[], kind: TokenKind, text: string) {
  if (!text) return;
  const last = out[out.length - 1];
  if (last && last.kind === kind) last.text += text;
  else out.push({ kind, text });
}

function tokenizeMarkup(src: string): Token[] {
  const out: Token[] = [];
  let i = 0;
  while (i < src.length) {
    if (src.startsWith('<!--', i)) {
      const end = src.indexOf('-->', i + 4);
      const j = end < 0 ? src.length : end + 3;
      push(out, 'comment', src.slice(i, j));
      i = j;
      continue;
    }
    if (src[i] === '<' && /[A-Za-z/!?]/.test(src[i + 1] ?? '')) {
      const m = /^<\/?[!?]?[\w:.-]*/.exec(src.slice(i));
      const tag = m ? m[0] : '<';
      push(out, 'tag', tag);
      i += tag.length;
      // attributes until '>'
      while (i < src.length && src[i] !== '>') {
        const rest = src.slice(i);
        const attr = /^[\w:@.#-]+/.exec(rest);
        if (attr) {
          push(out, 'attr', attr[0]);
          i += attr[0].length;
          continue;
        }
        if (src[i] === '"' || src[i] === "'") {
          const q = src[i];
          const end = src.indexOf(q, i + 1);
          const j = end < 0 ? src.length : end + 1;
          push(out, 'string', src.slice(i, j));
          i = j;
          continue;
        }
        if (src[i] === '{') {
          const end = src.indexOf('}', i);
          const j = end < 0 ? src.length : end + 1;
          push(out, 'var', src.slice(i, j));
          i = j;
          continue;
        }
        push(out, src[i] === '=' || src[i] === '/' ? 'punct' : 'plain', src[i]);
        i++;
      }
      if (src[i] === '>') {
        push(out, 'tag', '>');
        i++;
      }
      continue;
    }
    const next = src.slice(i).search(/<|\{/);
    if (src[i] === '{') {
      const end = src.indexOf('}', i);
      const j = end < 0 ? i + 1 : end + 1;
      push(out, 'var', src.slice(i, j));
      i = j;
      continue;
    }
    const j = next <= 0 ? (next === 0 ? i + 1 : src.length) : i + next;
    push(out, 'plain', src.slice(i, j));
    i = j;
  }
  return out;
}

function tokenizeLang(src: string, L: Lang): Token[] {
  const out: Token[] = [];
  let i = 0;
  const n = src.length;
  const idStart = /[A-Za-z_$@]/;
  const idChar = /[\w$]/;
  outer: while (i < n) {
    const c = src[i];
    for (const lc of L.line ?? []) {
      if (src.startsWith(lc, i) && !(lc === '#' && L === SH && i > 0 && !/\s/.test(src[i - 1]))) {
        const end = src.indexOf('\n', i);
        const j = end < 0 ? n : end;
        push(out, 'comment', src.slice(i, j));
        i = j;
        continue outer;
      }
    }
    for (const [open, close] of L.block ?? []) {
      if (src.startsWith(open, i)) {
        const end = src.indexOf(close, i + open.length);
        const j = end < 0 ? n : end + close.length;
        push(out, 'comment', src.slice(i, j));
        i = j;
        continue outer;
      }
    }
    if (L.triple && (src.startsWith('"""', i) || src.startsWith("'''", i))) {
      const q = src.slice(i, i + 3);
      const end = src.indexOf(q, i + 3);
      const j = end < 0 ? n : end + 3;
      push(out, 'string', src.slice(i, j));
      i = j;
      continue;
    }
    if (L.rustChars && c === "'") {
      // 'a' / '\n' are chars; 'a (no closing quote soon) is a lifetime.
      const m = /^'(?:\\.|[^'\\])'/.exec(src.slice(i, i + 12));
      if (m) {
        push(out, 'string', m[0]);
        i += m[0].length;
      } else {
        const lt = /^'[A-Za-z_]\w*/.exec(src.slice(i));
        const t = lt ? lt[0] : c;
        push(out, lt ? 'type' : 'punct', t);
        i += t.length;
      }
      continue;
    }
    if (L.strings.includes(c)) {
      let j = i + 1;
      while (j < n && src[j] !== c) {
        if (src[j] === '\\') j++;
        else if (src[j] === '\n' && c !== '`') break;
        j++;
      }
      j = Math.min(n, j + 1);
      push(out, 'string', src.slice(i, j));
      i = j;
      continue;
    }
    if (L.dollarVars && c === '$') {
      const m = /^\$(?:\{[^}]*\}|[\w@#?*!$-]+)/.exec(src.slice(i));
      if (m) {
        push(out, 'var', m[0]);
        i += m[0].length;
        continue;
      }
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] ?? ''))) {
      if (i === 0 || !idChar.test(src[i - 1])) {
        const m = /^(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?|\.\d+)[a-zA-Z0-9_]*/.exec(src.slice(i));
        if (m) {
          push(out, 'number', m[0]);
          i += m[0].length;
          continue;
        }
      }
    }
    if (idStart.test(c)) {
      const m = /^[A-Za-z_$@][\w$-]*/.exec(src.slice(i));
      let w = m ? m[0] : c;
      // Hyphens only belong to identifiers in CSS/shell-ish languages.
      if (L !== CSS && L !== SH && L !== YAML) w = w.split('-')[0] || c;
      const key = L.ci ? w.toLowerCase() : w;
      let kind: TokenKind = 'plain';
      if (L.keywords.has(key)) kind = 'keyword';
      else if (L.literals?.has(key)) kind = 'number';
      else if (L.types?.has(key)) kind = 'type';
      else if (src[i + w.length] === '(' || (L === RUST && src[i + w.length] === '!')) kind = 'fn';
      else if (/^[A-Z][a-z0-9]/.test(w) && L !== SQL) kind = 'type';
      else if (L === YAML && /^\s*:/.test(src.slice(i + w.length, i + w.length + 3))) kind = 'attr';
      else if (L === CSS && /^\s*:/.test(src.slice(i + w.length, i + w.length + 3))) kind = 'attr';
      push(out, kind, w);
      i += w.length;
      continue;
    }
    push(out, /[{}()[\];,.:=<>+\-*/%!&|^~?]/.test(c) ? 'punct' : 'plain', c);
    i++;
  }
  // JSON: strings followed by ':' are keys.
  if (L === JSON_L) {
    for (let k = 0; k < out.length; k++) {
      if (out[k].kind === 'string') {
        let m = k + 1;
        while (m < out.length && out[m].kind === 'plain' && !out[m].text.trim()) m++;
        if (out[m]?.kind === 'punct' && out[m].text.startsWith(':')) out[k] = { kind: 'attr', text: out[k].text };
      }
    }
  }
  return out;
}

/** Tokens for `src` in `lang`; unknown languages come back as one plain token. */
function tokenize(src: string, lang: string | null): Token[] {
  const L = lang ? LANGS[lang] : undefined;
  if (!L) return src ? [{ kind: 'plain', text: src }] : [];
  // Very large blocks stay plain: highlighting is a nicety, not worth a stall.
  if (src.length > 200_000) return [{ kind: 'plain', text: src }];
  return L === 'markup' ? tokenizeMarkup(src) : tokenizeLang(src, L);
}

/** Fills `code` with highlighted spans (DOM only, no innerHTML). */
export function highlightInto(code: HTMLElement, src: string, lang: string | null): void {
  const doc = code.ownerDocument;
  const frag = doc.createDocumentFragment();
  for (const t of tokenize(src, lang)) {
    if (t.kind === 'plain') {
      frag.appendChild(doc.createTextNode(t.text));
    } else {
      const s = doc.createElement('span');
      s.className = `tok-${t.kind}`;
      s.textContent = t.text;
      frag.appendChild(s);
    }
  }
  code.replaceChildren(frag);
}
