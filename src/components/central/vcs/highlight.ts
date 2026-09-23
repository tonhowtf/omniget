// Destaque de sintaxe leve por regex, sem lib: comentários, strings,
// números, palavras-chave, tipos (Maiúscula) e chamadas de função. Uma linha
// por vez, com o estado de comentário de bloco carregado entre linhas. Saída
// em HTML já escapado.

export type HlState = { block: boolean };

type Lang = {
  line: string[];
  block: [string, string] | null;
  quotes: string;
  keywords: Set<string>;
  /** `#` inicia comentário só no começo de palavra (shell/yaml/python). */
};

const words = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const JS_KW =
  "abstract as async await break case catch class const continue debugger declare default delete do else enum export extends false finally for from function get if implements import in infer instanceof interface is keyof let namespace new null of package private protected public readonly return satisfies set static super switch this throw true try type typeof undefined unique var void while with yield";

const C_LIKE =
  "auto bool break case catch char class const constexpr continue default delete do double else enum explicit extern false final float for friend goto if inline int long namespace new noexcept nullptr operator override private protected public register return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile while";

const LANGS: Record<string, Lang> = {
  ts: { line: ["//"], block: ["/*", "*/"], quotes: "\"'`", keywords: words(JS_KW) },
  rust: {
    line: ["//"],
    block: ["/*", "*/"],
    quotes: '"',
    keywords: words(
      "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while Some None Ok Err",
    ),
  },
  python: {
    line: ["#"],
    block: null,
    quotes: "\"'",
    keywords: words(
      "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield match case",
    ),
  },
  go: {
    line: ["//"],
    block: ["/*", "*/"],
    quotes: "\"'`",
    keywords: words(
      "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil iota",
    ),
  },
  c: { line: ["//"], block: ["/*", "*/"], quotes: "\"'", keywords: words(C_LIKE) },
  java: {
    line: ["//"],
    block: ["/*", "*/"],
    quotes: "\"'",
    keywords: words(
      C_LIKE +
        " abstract assert boolean byte extends final finally implements import instanceof interface native package super synchronized throws transient var record sealed permits val fun when object companion data suspend lateinit internal open is in out",
    ),
  },
  csharp: {
    line: ["//"],
    block: ["/*", "*/"],
    quotes: "\"'",
    keywords: words(
      C_LIKE + " abstract base checked decimal event fixed foreach implicit in interface internal is lock null object out params readonly ref sbyte sealed string uint ulong ushort var async await get set",
    ),
  },
  swift: {
    line: ["//"],
    block: ["/*", "*/"],
    quotes: '"',
    keywords: words(
      "associatedtype class deinit enum extension fileprivate func import init inout internal let open operator private protocol public static struct subscript typealias var break case continue default defer do else fallthrough for guard if in repeat return switch where while as catch false is nil rethrows super self Self throw throws true try async await some any",
    ),
  },
  ruby: {
    line: ["#"],
    block: null,
    quotes: "\"'",
    keywords: words(
      "alias and begin break case class def defined? do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield require",
    ),
  },
  php: { line: ["//", "#"], block: ["/*", "*/"], quotes: "\"'", keywords: words(C_LIKE + " function echo array fn match use namespace foreach elseif endif") },
  bash: {
    line: ["#"],
    block: null,
    quotes: "\"'",
    keywords: words("if then else elif fi case esac for while until do done in function return local export readonly declare set unset shift exit source echo"),
  },
  yaml: { line: ["#"], block: null, quotes: "\"'", keywords: words("true false null yes no on off") },
  toml: { line: ["#"], block: null, quotes: "\"'", keywords: words("true false") },
  json: { line: [], block: null, quotes: '"', keywords: words("true false null") },
  css: {
    line: [],
    block: ["/*", "*/"],
    quotes: "\"'",
    keywords: words("important media import supports keyframes font-face from to and not only screen root"),
  },
  html: { line: [], block: ["<!--", "-->"], quotes: "\"'", keywords: new Set() },
  sql: {
    line: ["--"],
    block: ["/*", "*/"],
    quotes: "'\"",
    keywords: words(
      "select from where and or not insert into values update set delete create table index view drop alter add primary key foreign references join left right inner outer on group by order having limit offset as distinct union all null is in like between case when then else end returning with SELECT FROM WHERE AND OR NOT INSERT INTO VALUES UPDATE SET DELETE CREATE TABLE INDEX DROP ALTER JOIN LEFT INNER ON GROUP BY ORDER HAVING LIMIT AS NULL IS IN CASE WHEN THEN ELSE END WITH",
    ),
  },
  lua: { line: ["--"], block: null, quotes: "\"'", keywords: words("and break do else elseif end false for function goto if in local nil not or repeat return then true until while") },
};

const ALIASES: Record<string, string> = {
  tsx: "ts", js: "ts", jsx: "ts", svelte: "ts", vue: "ts", kotlin: "java", scala: "java", dart: "java",
  cpp: "c", scss: "css", xml: "html", markdown: "", ini: "toml", powershell: "bash", dockerfile: "bash", makefile: "bash",
};

export function langFor(language: string): Lang | null {
  const key = ALIASES[language] ?? language;
  return LANGS[key] ?? null;
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : c === ">" ? "&gt;" : "&quot;"));
}

const span = (cls: string, s: string) => `<span class="hl-${cls}">${escapeHtml(s)}</span>`;

const ID_START = /[A-Za-z_$@]/;
const ID_CHAR = /[\w$?!-]/;

/** Destaca uma linha. `state` é lido e atualizado (comentário de bloco aberto). */
export function highlightLine(text: string, language: string, state: HlState): string {
  const lang = langFor(language);
  if (!lang || text.length > 2000) return escapeHtml(text);
  let out = "";
  let i = 0;
  const n = text.length;
  if (state.block && lang.block) {
    const end = text.indexOf(lang.block[1]);
    if (end < 0) return span("comment", text);
    out += span("comment", text.slice(0, end + lang.block[1].length));
    i = end + lang.block[1].length;
    state.block = false;
  }
  let plain = "";
  const flush = () => {
    if (plain) {
      out += escapeHtml(plain);
      plain = "";
    }
  };
  while (i < n) {
    const rest = text.slice(i);
    // comentário de linha
    const lc = lang.line.find((p) => rest.startsWith(p) && (p !== "#" || i === 0 || /\s/.test(text[i - 1])));
    if (lc) {
      flush();
      out += span("comment", rest);
      return out;
    }
    // comentário de bloco
    if (lang.block && rest.startsWith(lang.block[0])) {
      flush();
      const end = text.indexOf(lang.block[1], i + lang.block[0].length);
      if (end < 0) {
        out += span("comment", rest);
        state.block = true;
        return out;
      }
      out += span("comment", text.slice(i, end + lang.block[1].length));
      i = end + lang.block[1].length;
      continue;
    }
    const c = text[i];
    // string
    if (lang.quotes.includes(c)) {
      flush();
      let j = i + 1;
      while (j < n && text[j] !== c) j += text[j] === "\\" ? 2 : 1;
      out += span("string", text.slice(i, Math.min(j + 1, n)));
      i = j + 1;
      continue;
    }
    // número
    if (/[0-9]/.test(c) && (i === 0 || !ID_CHAR.test(text[i - 1]))) {
      flush();
      const m = /^(0x[0-9a-fA-F_]+|0b[01_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?)[a-zA-Z0-9]*/.exec(rest);
      const s = m ? m[0] : c;
      out += span("number", s);
      i += s.length;
      continue;
    }
    // identificador
    if (ID_START.test(c) && (i === 0 || !ID_CHAR.test(text[i - 1]))) {
      let j = i + 1;
      while (j < n && /[\w$]/.test(text[j])) j++;
      const word = text.slice(i, j);
      flush();
      if (lang.keywords.has(word)) out += span("keyword", word);
      else if (c === "@") out += span("meta", word);
      else if (/^[A-Z][A-Za-z0-9_]*$/.test(word) && word.length > 1) out += span("type", word);
      else if (text[j] === "(" || (text[j] === "!" && language === "rust")) out += span("fn", word);
      else out += escapeHtml(word);
      i = j;
      continue;
    }
    plain += c;
    i++;
  }
  flush();
  return out;
}
