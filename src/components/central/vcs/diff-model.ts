// Modelo do DiffViewer: parser de patch unificado (feito aqui, sem lib),
// linhas numeradas, pareamento para a vista dividida, dobra de contexto e o
// mini hunk de um comentário de revisão.

import type { FileDiff, ReviewComment } from "$lib/central/vcs";

export type LineKind = "ctx" | "add" | "del";

export type DiffLine = {
  kind: LineKind;
  /** Número na versão antiga (ctx/del). */
  old: number | null;
  /** Número na versão nova (ctx/add). */
  new: number | null;
  text: string;
  /** "\ No newline at end of file" veio logo depois. */
  noEol: boolean;
};

export type Hunk = {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** Texto depois do segundo @@ (nome de função, quando o git dá). */
  section: string;
  lines: DiffLine[];
};

export type ParsedPatch = {
  hunks: Hunk[];
  binary: boolean;
  /** Linhas de cabeçalho (mode, rename, index…). */
  meta: string[];
};

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$/;

export function parsePatch(patch: string): ParsedPatch {
  const out: ParsedPatch = { hunks: [], binary: false, meta: [] };
  if (!patch) return out;
  const lines = patch.split("\n");
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  let cur: Hunk | null = null;
  let o = 0;
  let n = 0;
  for (const raw of lines) {
    const m = HUNK_RE.exec(raw);
    if (m) {
      cur = {
        header: raw,
        oldStart: +m[1],
        oldLines: m[2] === undefined ? 1 : +m[2],
        newStart: +m[3],
        newLines: m[4] === undefined ? 1 : +m[4],
        section: m[5] ?? "",
        lines: [],
      };
      out.hunks.push(cur);
      o = cur.oldStart;
      n = cur.newStart;
      continue;
    }
    if (!cur) {
      if (raw.startsWith("Binary files ") || raw.startsWith("GIT binary patch")) out.binary = true;
      if (!raw.startsWith("diff --git ") && !raw.startsWith("--- ") && !raw.startsWith("+++ ")) out.meta.push(raw);
      continue;
    }
    const c = raw[0];
    const text = raw.slice(1).replace(/\r$/, "");
    if (c === "+") cur.lines.push({ kind: "add", old: null, new: n++, text, noEol: false });
    else if (c === "-") cur.lines.push({ kind: "del", old: o++, new: null, text, noEol: false });
    else if (c === " " || raw === "") cur.lines.push({ kind: "ctx", old: o++, new: n++, text, noEol: false });
    else if (c === "\\") {
      const last = cur.lines[cur.lines.length - 1];
      if (last) last.noEol = true;
    } else if (raw.startsWith("diff --git ")) {
      cur = null;
    }
  }
  return out;
}

// ---- linhas renderizáveis ----

export type Row =
  | { t: "file"; fileIndex: number }
  | { t: "gap"; fileIndex: number; hidden: number; before: boolean }
  | { t: "hunk"; fileIndex: number; hunk: number; label: string }
  | { t: "fold"; fileIndex: number; key: string; hidden: number }
  | { t: "line"; fileIndex: number; line: DiffLine; id: number }
  | { t: "pair"; fileIndex: number; left: DiffLine | null; right: DiffLine | null; id: number }
  | { t: "note"; fileIndex: number; text: string };

/** Contexto mantido de cada lado de um trecho dobrado. */
export const FOLD_KEEP = 3;
/** Trechos de contexto maiores que isto dobram. */
export const FOLD_MIN = FOLD_KEEP * 2 + 4;

type Block = { kind: "ctx"; lines: DiffLine[] } | { kind: "chg"; dels: DiffLine[]; adds: DiffLine[] };

function blocks(lines: DiffLine[]): Block[] {
  const out: Block[] = [];
  for (const l of lines) {
    const last = out[out.length - 1];
    if (l.kind === "ctx") {
      if (last && last.kind === "ctx") last.lines.push(l);
      else out.push({ kind: "ctx", lines: [l] });
    } else {
      let b: Block;
      if (last && last.kind === "chg" && !(l.kind === "del" && last.adds.length)) b = last;
      else {
        b = { kind: "chg", dels: [], adds: [] };
        out.push(b);
      }
      if (l.kind === "del") b.dels.push(l);
      else b.adds.push(l);
    }
  }
  return out;
}

/**
 * Linhas de um arquivo para a vista `unified` ou `split`. `expanded` guarda as
 * chaves de dobras abertas. `idBase` numera as linhas de forma estável para a
 * seleção.
 */
export function fileRows(
  fileIndex: number,
  parsed: ParsedPatch,
  mode: "unified" | "split",
  expanded: Set<string>,
): Row[] {
  const rows: Row[] = [];
  let id = 0;
  parsed.hunks.forEach((h, hi) => {
    if (hi === 0 && h.oldStart > 1) rows.push({ t: "gap", fileIndex, hidden: h.oldStart - 1, before: true });
    if (hi > 0) {
      const prev = parsed.hunks[hi - 1];
      const hidden = h.oldStart - (prev.oldStart + prev.oldLines);
      if (hidden > 0) rows.push({ t: "gap", fileIndex, hidden, before: false });
    }
    rows.push({ t: "hunk", fileIndex, hunk: hi, label: h.header });
    const bs = blocks(h.lines);
    bs.forEach((b, bi) => {
      if (b.kind === "ctx") {
        const key = `${hi}:${bi}`;
        const first = bi === 0;
        const last = bi === bs.length - 1;
        // Contexto de abertura mantém só o fim (perto da mudança), o de
        // fechamento só o começo, o do meio as duas pontas.
        const keepHead = first && !last ? 0 : FOLD_KEEP;
        const keepTail = last && !first ? 0 : FOLD_KEEP;
        const hiddenCount = b.lines.length - keepHead - keepTail;
        const fold = b.lines.length >= FOLD_MIN && hiddenCount > 2 && !expanded.has(key);
        const pushCtx = (l: DiffLine) =>
          mode === "unified"
            ? rows.push({ t: "line", fileIndex, line: l, id: id++ })
            : rows.push({ t: "pair", fileIndex, left: l, right: l, id: id++ });
        if (!fold) {
          b.lines.forEach(pushCtx);
        } else {
          b.lines.slice(0, keepHead).forEach(pushCtx);
          rows.push({ t: "fold", fileIndex, key, hidden: hiddenCount });
          id += hiddenCount;
          b.lines.slice(b.lines.length - keepTail).forEach(pushCtx);
        }
      } else if (mode === "unified") {
        for (const l of b.dels) rows.push({ t: "line", fileIndex, line: l, id: id++ });
        for (const l of b.adds) rows.push({ t: "line", fileIndex, line: l, id: id++ });
      } else {
        const n = Math.max(b.dels.length, b.adds.length);
        for (let i = 0; i < n; i++)
          rows.push({ t: "pair", fileIndex, left: b.dels[i] ?? null, right: b.adds[i] ?? null, id: id++ });
      }
    });
  });
  return rows;
}

// ---- comentário de revisão ----

export function languageOf(path: string): string {
  const name = path.split("/").pop() ?? path;
  const lower = name.toLowerCase();
  if (lower === "dockerfile") return "dockerfile";
  if (lower === "makefile") return "makefile";
  const ext = lower.includes(".") ? lower.split(".").pop()! : "";
  const map: Record<string, string> = {
    ts: "ts", mts: "ts", cts: "ts", tsx: "tsx", js: "js", mjs: "js", cjs: "js", jsx: "jsx",
    svelte: "svelte", vue: "vue", rs: "rust", py: "python", go: "go", java: "java", kt: "kotlin",
    kts: "kotlin", c: "c", h: "c", cc: "cpp", cpp: "cpp", hpp: "cpp", cs: "csharp", swift: "swift",
    rb: "ruby", php: "php", json: "json", jsonc: "json", css: "css", scss: "scss", less: "css",
    html: "html", htm: "html", xml: "xml", svg: "xml", sh: "bash", bash: "bash", zsh: "bash",
    fish: "bash", ps1: "powershell", yml: "yaml", yaml: "yaml", toml: "toml", ini: "ini",
    md: "markdown", mdx: "markdown", sql: "sql", lua: "lua", dart: "dart", scala: "scala",
  };
  return map[ext] ?? "text";
}

function fence(content: string): string {
  const longest = Math.max(2, ...Array.from(content.matchAll(/`+/g), (m) => m[0].length));
  return "`".repeat(longest + 1);
}

/** Cerca segura para embutir `content` num prompt. */
export function fenced(content: string, language = ""): string {
  const f = fence(content);
  return `${f}${language}\n${content}\n${f}`;
}

/** Monta o `ReviewComment` de um conjunto de linhas selecionadas. */
export function buildReviewComment(path: string, lines: DiffLine[], text: string): ReviewComment {
  const allDel = lines.every((l) => l.kind === "del");
  const allAdd = lines.every((l) => l.kind === "add");
  const side: ReviewComment["side"] = allDel ? "old" : lines.some((l) => l.kind === "del") && !allDel ? "mixed" : "new";
  const nums = lines.map((l) => (allDel ? l.old : (l.new ?? l.old)) ?? 0);
  const start = Math.min(...nums);
  const end = Math.max(...nums);
  const mark = allDel ? "-" : allAdd ? "+" : "L";
  const rangeLabel = start === end ? `${mark}${start}` : `${mark}${start} to ${mark}${end}`;
  const olds = lines.filter((l) => l.old !== null).map((l) => l.old!);
  const news = lines.filter((l) => l.new !== null).map((l) => l.new!);
  const oStart = olds.length ? Math.min(...olds) : 0;
  const nStart = news.length ? Math.min(...news) : 0;
  const header = `@@ -${oStart},${olds.length} +${nStart},${news.length} @@`;
  const body = lines.map((l) => (l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ") + l.text).join("\n");
  return { path, side, start, end, rangeLabel, diff: `${header}\n${body}`, text, language: languageOf(path) };
}

/** Texto pronto para colar no composer. */
export function reviewCommentToPrompt(c: ReviewComment): string {
  const head = `${c.path} (${c.rangeLabel})`;
  const code = fenced(c.diff, "diff");
  return c.text.trim() ? `${head}\n${code}\n${c.text.trim()}` : `${head}\n${code}`;
}

export function statsOf(f: FileDiff): { add: number; del: number } {
  return { add: f.additions, del: f.deletions };
}
