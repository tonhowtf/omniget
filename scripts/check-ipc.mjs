#!/usr/bin/env node

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, relative, dirname, extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const asJson = argv.includes("--json");
const verbose = argv.includes("--verbose");

const SKIP_DIRS = new Set(["node_modules", "target", ".svelte-kit", "build", ".git", "gen", "binaries", "__mocks__"]);
const rel = (p) => relative(ROOT, p).split("\\").join("/");

function walk(dir, exts, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(e.name)) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, exts, out);
    else if (exts.includes(extname(e.name))) out.push(p);
  }
  return out;
}

const REGEX_PREV = new Set(["", "(", ",", "=", ":", "[", "!", "&", "|", "?", "{", "}", ";", "+", "-", "*", "%", "<", ">", "~", "^"]);
const REGEX_WORDS = new Set(["return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw", "yield", "await"]);

function lexJs(src, from, to, code, mask, stopAtClose = false) {
  const blank = (a, b, keepCode = false) => {
    for (let k = a; k < b; k++) {
      if (src[k] === "\n") continue;
      mask[k] = " ";
      if (!keepCode) code[k] = " ";
    }
  };
  let i = from;
  let depth = 0;
  const tpl = [];
  let prev = "";
  let prevWord = "";
  const scanTemplate = () => {
    while (i < to) {
      const c = src[i];
      if (c === "\\") {
        blank(i, Math.min(i + 2, to), true);
        i += 2;
        continue;
      }
      if (c === "`") {
        i++;
        prev = "str";
        return;
      }
      if (c === "$" && src[i + 1] === "{") {
        tpl.push(depth);
        i += 2;
        prev = "{";
        return;
      }
      blank(i, i + 1, true);
      i++;
    }
  };
  while (i < to) {
    const c = src[i];
    const n = src[i + 1];
    if (c === "/" && n === "/") {
      const e = src.indexOf("\n", i);
      const end = e < 0 || e > to ? to : e;
      blank(i, end);
      i = end;
      continue;
    }
    if (c === "/" && n === "*") {
      const e = src.indexOf("*/", i + 2);
      const end = e < 0 || e + 2 > to ? to : e + 2;
      blank(i, end);
      i = end;
      continue;
    }
    if (c === '"' || c === "'") {
      let j = i + 1;
      while (j < to && src[j] !== c && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1;
      blank(i + 1, Math.min(j, to), true);
      i = j + 1;
      prev = "str";
      prevWord = "";
      continue;
    }
    if (c === "`") {
      i++;
      scanTemplate();
      prevWord = "";
      continue;
    }
    if (c === "/" && (REGEX_PREV.has(prev) || REGEX_WORDS.has(prevWord))) {
      let j = i + 1;
      let cls = false;
      while (j < to && src[j] !== "\n") {
        if (src[j] === "\\") {
          j += 2;
          continue;
        }
        if (src[j] === "[") cls = true;
        else if (src[j] === "]") cls = false;
        else if (src[j] === "/" && !cls) break;
        j++;
      }
      blank(i + 1, Math.min(j, to), true);
      i = j + 1;
      while (i < to && /[a-z]/i.test(src[i])) i++;
      prev = "re";
      prevWord = "";
      continue;
    }
    if (c === "{") depth++;
    if (c === "}") {
      if (tpl.length && tpl[tpl.length - 1] === depth) {
        tpl.pop();
        i++;
        scanTemplate();
        continue;
      }
      if (depth === 0 && stopAtClose) return i;
      depth--;
    }
    if (/[A-Za-z_$0-9]/.test(c)) {
      let j = i;
      while (j < to && /[A-Za-z_$0-9]/.test(src[j])) j++;
      prevWord = src.slice(i, j);
      prev = "w";
      i = j;
      continue;
    }
    if (!/\s/.test(c)) {
      prev = c;
      prevWord = "";
    }
    i++;
  }
  return to;
}

function lexScript(src) {
  const code = src.split("");
  const mask = src.split("");
  lexJs(src, 0, src.length, code, mask);
  return { code: code.join(""), mask: mask.join("") };
}

function lexSvelte(src) {
  const code = src.split("").map((ch) => (ch === "\n" ? "\n" : " "));
  const mask = code.slice();
  const copy = (a, b) => {
    for (let k = a; k < b; k++) code[k] = mask[k] = src[k];
  };
  let i = 0;
  while (i < src.length) {
    if (src.startsWith("<!--", i)) {
      const e = src.indexOf("-->", i);
      i = e < 0 ? src.length : e + 3;
      continue;
    }
    const m = src[i] === "<" ? /^<(script|style)\b[^>]*>/.exec(src.slice(i, i + 200)) : null;
    if (m) {
      const close = src.indexOf(`</${m[1]}>`, i);
      const bodyStart = i + m[0].length;
      const bodyEnd = close < 0 ? src.length : close;
      if (m[1] === "script") {
        copy(bodyStart, bodyEnd);
        lexJs(src, bodyStart, bodyEnd, code, mask);
      }
      i = bodyEnd;
      continue;
    }
    if (src[i] === "{") {
      if (src[i + 1] === "/") {
        const e = src.indexOf("}", i);
        i = e < 0 ? src.length : e + 1;
        continue;
      }
      const start = i + 1;
      copy(start, src.length);
      const end = lexJs(src, start, src.length, code, mask, true);
      for (let k = end; k < src.length; k++) code[k] = mask[k] = src[k] === "\n" ? "\n" : " ";
      i = end + 1;
      continue;
    }
    i++;
  }
  return { code: code.join(""), mask: mask.join("") };
}

function lexRust(src) {
  const code = src.split("");
  const mask = src.split("");
  const blank = (a, b, keepCode = false) => {
    for (let k = a; k < b; k++) {
      if (src[k] === "\n") continue;
      mask[k] = " ";
      if (!keepCode) code[k] = " ";
    }
  };
  let i = 0;
  const L = src.length;
  while (i < L) {
    const c = src[i];
    const n = src[i + 1];
    if (c === "/" && n === "/") {
      const e = src.indexOf("\n", i);
      const end = e < 0 ? L : e;
      blank(i, end);
      i = end;
      continue;
    }
    if (c === "/" && n === "*") {
      let d = 1;
      let j = i + 2;
      while (j < L && d > 0) {
        if (src[j] === "/" && src[j + 1] === "*") {
          d++;
          j += 2;
        } else if (src[j] === "*" && src[j + 1] === "/") {
          d--;
          j += 2;
        } else j++;
      }
      blank(i, j);
      i = j;
      continue;
    }
    const raw = /^(b?r)(#*)"/.exec(src.slice(i, i + 12));
    if (raw && !/[A-Za-z0-9_]/.test(src[i - 1] ?? "")) {
      const hashes = raw[2];
      const bodyStart = i + raw[0].length;
      const e = src.indexOf(`"${hashes}`, bodyStart);
      const end = e < 0 ? L : e;
      blank(bodyStart, end, true);
      i = end + 1 + hashes.length;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      while (j < L && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
      blank(i + 1, j, true);
      i = j + 1;
      continue;
    }
    if (c === "'") {
      const ch = /^'(\\u\{[0-9a-fA-F]+\}|\\.|[^\\'])'/u.exec(src.slice(i, i + 12));
      if (ch) {
        blank(i + 1, i + ch[0].length - 1, true);
        i += ch[0].length;
        continue;
      }
    }
    i++;
  }
  return { code: code.join(""), mask: mask.join("") };
}

const OPEN = { "(": ")", "[": "]", "{": "}" };
const CLOSE = new Set([")", "]", "}"]);

function skipGeneric(mask, i) {
  let d = 0;
  let inner = 0;
  for (let j = i; j < mask.length && j < i + 4000; j++) {
    const c = mask[j];
    if (OPEN[c]) inner++;
    else if (CLOSE.has(c)) {
      inner--;
      if (inner < 0) return -1;
    } else if (inner === 0 && c === "<") d++;
    else if (inner === 0 && c === ">" && mask[j - 1] !== "=") {
      d--;
      if (d === 0) return j + 1;
    } else if (inner === 0 && c === ";") return -1;
  }
  return -1;
}

function splitArgs(mask, open) {
  const args = [];
  let d = 0;
  let start = open + 1;
  for (let j = open; j < mask.length; j++) {
    const c = mask[j];
    if (OPEN[c]) d++;
    else if (CLOSE.has(c)) {
      d--;
      if (d === 0) {
        if (mask.slice(start, j).trim()) args.push([start, j]);
        return { args, end: j };
      }
    } else if (c === "," && d === 1) {
      args.push([start, j]);
      start = j + 1;
    }
  }
  return { args, end: mask.length };
}

function matchBrace(mask, open) {
  let d = 0;
  for (let j = open; j < mask.length; j++) {
    if (OPEN[mask[j]]) d++;
    else if (CLOSE.has(mask[j])) {
      d--;
      if (d === 0) return j;
    }
  }
  return mask.length;
}

const lineOf = (text, index) => {
  let n = 1;
  for (let k = 0; k < index && k < text.length; k++) if (text.charCodeAt(k) === 10) n++;
  return n;
};

const frontFiles = walk(join(ROOT, "src"), [".ts", ".js", ".mjs", ".svelte"]).filter(
  (f) => !/\.(test|spec)\.(ts|js|mjs)$/.test(f) && !f.endsWith(".d.ts"),
);

const files = new Map();
for (const abs of frontFiles) {
  const src = readFileSync(abs, "utf8");
  const { code, mask } = abs.endsWith(".svelte") ? lexSvelte(src) : lexScript(src);
  files.set(abs, { abs, path: rel(abs), src, code, mask });
}

function resolveModule(fromAbs, spec) {
  let base;
  if (spec.startsWith("$lib/")) base = join(ROOT, "src", "lib", spec.slice(5));
  else if (spec === "$lib") base = join(ROOT, "src", "lib");
  else if (spec.startsWith("$components/")) base = join(ROOT, "src", "components", spec.slice(12));
  else if (spec.startsWith(".")) base = resolve(dirname(fromAbs), spec);
  else return null;
  const cands = [base, `${base}.ts`, `${base}.js`, `${base}.svelte.ts`, `${base}.svelte`, join(base, "index.ts"), join(base, "index.js")];
  if (base.endsWith(".js")) cands.push(base.replace(/\.js$/, ".ts"));
  return cands.find((c) => files.has(c)) ?? null;
}

function parseImports(f) {
  const out = new Map();
  const re = /\bimport\s+(?:type\s+)?\{([^}]*)\}\s*from\s*(["'])([^"']+)\2/g;
  let m;
  while ((m = re.exec(f.code))) {
    const target = resolveModule(f.abs, m[3]);
    for (const part of m[1].split(",")) {
      const p = part.trim().replace(/^type\s+/, "");
      if (!p) continue;
      const [orig, local] = p.split(/\s+as\s+/).map((s) => s.trim());
      out.set(local ?? orig, { file: target, name: orig, spec: m[3] });
    }
  }
  return out;
}

function stringValue(text) {
  const t = text.trim();
  const m = /^(["'])([^"'\\\n]*)\1$/.exec(t);
  if (m) return { exact: m[2] };
  const tm = /^`([^`]*)`$/.exec(t);
  if (tm) {
    const k = tm[1].indexOf("${");
    if (k < 0) return { exact: tm[1] };
    return { prefix: tm[1].slice(0, k) };
  }
  return null;
}

function findConst(f, name) {
  const re = new RegExp(`(?<![\\w$.])(?:const|let|var)\\s+${name}\\s*(?::[^=]+)?=\\s*`, "g");
  let m;
  while ((m = re.exec(f.mask))) {
    const start = m.index + m[0].length;
    const endM = /[;\n]/.exec(f.mask.slice(start));
    const end = endM ? start + endM.index : f.mask.length;
    const text = f.code.slice(start, end).replace(/\s+as\s+const\s*$/, "");
    const v = stringValue(text);
    if (v) return v;
  }
  return null;
}

function findObjectKey(f, obj, key) {
  const re = new RegExp(`(?<![\\w$.])(?:const|let|var)\\s+${obj}\\s*(?::[^=]+)?=\\s*\\{`, "g");
  const m = re.exec(f.mask);
  if (!m) return null;
  const open = m.index + m[0].length - 1;
  const close = matchBrace(f.mask, open);
  const body = f.mask.slice(open + 1, close);
  const km = new RegExp(`(?:^|[,{\\s])["']?${key}["']?\\s*:\\s*`).exec(body);
  if (!km) return null;
  const start = open + 1 + km.index + km[0].length;
  let d = 0;
  let end = start;
  for (; end < close; end++) {
    const c = f.mask[end];
    if (OPEN[c]) d++;
    else if (CLOSE.has(c)) d--;
    else if ((c === "," || c === "\n") && d === 0) break;
  }
  return stringValue(f.code.slice(start, end).replace(/\s+as\s+const\s*$/, ""));
}

function parseFunctions(f) {
  if (f.functions) return f.functions;
  const out = [];
  const mask = f.mask;
  const heads = [
    /\bfunction\s*\*?\s*([A-Za-z_$][\w$]*)?\s*(?=[<(])/g,
    /([A-Za-z_$][\w$]*)\s*[:=]\s*(?:async\s*)?(?=[<(])/g,
    /(?:^|[\s,{;])(?:async\s+|static\s+|public\s+|private\s+)*([A-Za-z_$][\w$]*)\s*(?=[<(])/g,
  ];
  const KEYWORDS = new Set(["if", "for", "while", "switch", "catch", "function", "return", "await", "typeof", "new", "import", "super", "with"]);
  for (const re of heads) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(mask))) {
      const name = m[1] ?? "";
      if (KEYWORDS.has(name)) continue;
      let j = m.index + m[0].length;
      if (mask[j] === "<") {
        j = skipGeneric(mask, j);
        if (j < 0) continue;
        while (/\s/.test(mask[j])) j++;
      }
      if (mask[j] !== "(") continue;
      const close = matchBrace(mask, j);
      const { args } = splitArgs(mask, j);
      let k = close + 1;
      while (/\s/.test(mask[k] ?? "")) k++;
      if (mask[k] === ":") {
        let d = 0;
        for (k = k + 1; k < mask.length; k++) {
          const c = mask[k];
          if (c === "<" || c === "(" || c === "[") d++;
          else if ((c === ">" && mask[k - 1] !== "=") || c === ")" || c === "]") d--;
          else if (c === "{" && d === 0 && /[\s)>\]]/.test(mask[k - 1])) {
            const before = mask.slice(0, k).trimEnd().slice(-1);
            if (!"|&<,:".includes(before)) break;
            k = matchBrace(mask, k);
          } else if (c === "{" && d === 0) k = matchBrace(mask, k);
          else if (c === "=" && mask[k + 1] === ">" && d === 0) break;
          else if (c === ";" && d === 0) break;
        }
      }
      let bodyStart;
      let bodyEnd;
      const isArrow = mask[k] === "=" && mask[k + 1] === ">";
      if (isArrow) {
        k += 2;
        while (/\s/.test(mask[k] ?? "")) k++;
      }
      if (mask[k] === "{") {
        bodyStart = k;
        bodyEnd = matchBrace(mask, k);
      } else if (isArrow) {
        bodyStart = k;
        let d = 0;
        for (bodyEnd = k; bodyEnd < mask.length; bodyEnd++) {
          const c = mask[bodyEnd];
          if (OPEN[c]) d++;
          else if (CLOSE.has(c)) {
            if (d === 0) break;
            d--;
          } else if ((c === ";" || c === ",") && d === 0) break;
        }
      } else continue;
      const params = args.map(([a, b]) => {
        const p = f.code.slice(a, b).trim().replace(/^\.\.\./, "");
        const pm = /^([A-Za-z_$][\w$]*)/.exec(p);
        return pm ? pm[1] : null;
      });
      out.push({ name, params, start: m.index, bodyStart, bodyEnd, method: re === heads[2] });
    }
  }
  f.functions = out;
  return out;
}

function enclosingWithParam(f, pos, ident) {
  let best = null;
  for (const fn of parseFunctions(f)) {
    if (pos <= fn.bodyStart || pos > fn.bodyEnd) continue;
    const idx = fn.params.indexOf(ident);
    if (idx < 0 || !fn.name) continue;
    if (!best || fn.bodyStart > best.bodyStart) best = { ...fn, index: idx };
  }
  return best;
}

function resolveExpr(f, a, b, depth = 0) {
  const text = f.code.slice(a, b).trim();
  const maskText = f.mask.slice(a, b);
  const lit = stringValue(text.replace(/\s+as\s+[\w$.<>[\]| ]+$/, ""));
  if (lit) return { names: [lit], dynamic: null };
  let d = 0;
  let q = -1;
  let colon = -1;
  for (let k = 0; k < maskText.length; k++) {
    const c = maskText[k];
    if (OPEN[c]) d++;
    else if (CLOSE.has(c)) d--;
    else if (d === 0 && c === "?" && maskText[k + 1] !== "?" && maskText[k + 1] !== "." && maskText[k - 1] !== "?") {
      if (q < 0) q = k;
    } else if (d === 0 && c === ":" && q >= 0 && colon < 0) colon = k;
  }
  if (q >= 0 && colon > q) {
    const l = resolveExpr(f, a + q + 1, a + colon, depth);
    const r = resolveExpr(f, a + colon + 1, b, depth);
    return { names: [...l.names, ...r.names], dynamic: l.dynamic ?? r.dynamic };
  }
  const bare = text.replace(/^\(+|\)+$/g, "").trim();
  const member = /^([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)$/.exec(bare);
  if (member) {
    const v = lookup(f, member[1], (file, name) => findObjectKey(file, name, member[2]));
    if (v) return { names: [v], dynamic: null };
    const swept = propertyLiterals(f, member[2]);
    if (swept.length) return { names: swept, dynamic: null };
    return { names: [], dynamic: bare };
  }
  const ident = /^([A-Za-z_$][\w$]*)$/.exec(bare);
  if (ident) {
    const loop = forOfBinding(f, a, ident[1]);
    if (loop) return loop;
    const fn = enclosingWithParam(f, a, ident[1]);
    if (fn && depth < 6) return { names: [], dynamic: null, wrapper: { file: f, fn } };
    const v = lookup(f, ident[1], (file, name) => findConst(file, name));
    if (v) return { names: [v], dynamic: null };
  }
  return { names: [], dynamic: bare.replace(/\s+/g, " ") };
}

function propertyLiterals(f, key) {
  const out = [];
  const re = new RegExp(`(?<![\\w$.])${key}\\s*:\\s*(?=["'\`])`, "g");
  let m;
  while ((m = re.exec(f.mask))) {
    const start = m.index + m[0].length;
    const q = f.mask[start];
    const end = f.mask.indexOf(q, start + 1);
    const v = stringValue(f.code.slice(start, end + 1));
    if (v?.exact && /^[a-z][a-z0-9_]*$/.test(v.exact)) out.push(v);
  }
  return out;
}

function forOfBinding(f, pos, ident) {
  const mask = f.mask;
  const re = /\bfor\s*\(\s*(?:const|let|var)\s+/g;
  let best = null;
  let m;
  while ((m = re.exec(mask)) && m.index < pos) {
    const open = mask.indexOf("(", m.index);
    const close = matchBrace(mask, open);
    let k = m.index + m[0].length;
    let slot = -1;
    let afterBinding;
    if (mask[k] === "[") {
      const pat = splitArgs(mask.slice(0, k) + "(" + mask.slice(k + 1), k);
      slot = pat.args.findIndex(([x, y]) => f.code.slice(x, y).trim() === ident);
      if (slot < 0) continue;
      afterBinding = pat.end + 1;
    } else {
      const id = /^([A-Za-z_$][\w$]*)/.exec(mask.slice(k));
      if (!id || id[1] !== ident) continue;
      afterBinding = k + id[1].length;
    }
    const of = /^\s*of\s*/.exec(mask.slice(afterBinding));
    if (!of) continue;
    const iterStart = afterBinding + of[0].length;
    if (mask[iterStart] !== "[") continue;
    let bodyStart = close + 1;
    while (/\s/.test(mask[bodyStart] ?? "")) bodyStart++;
    const bodyEnd = mask[bodyStart] === "{" ? matchBrace(mask, bodyStart) : mask.indexOf(";", bodyStart);
    if (pos < bodyStart || pos > bodyEnd) continue;
    const arr = splitArgs(mask.slice(0, iterStart) + "(" + mask.slice(iterStart + 1), iterStart);
    const names = [];
    for (const [x, y] of arr.args) {
      let text = f.code.slice(x, y).trim();
      if (slot >= 0) {
        const s0 = x + f.code.slice(x, y).indexOf("[");
        if (mask[s0] !== "[") return null;
        const inner = splitArgs(mask.slice(0, s0) + "(" + mask.slice(s0 + 1), s0).args[slot];
        if (!inner) return null;
        text = f.code.slice(inner[0], inner[1]);
      }
      const v = stringValue(text);
      if (!v) return null;
      names.push(v);
    }
    if (!best || m.index > best.at) best = { at: m.index, names };
  }
  return best ? { names: best.names, dynamic: null } : null;
}

function lookup(f, name, finder) {
  const local = finder(f, name);
  if (local) return local;
  f.imports ??= parseImports(f);
  const imp = f.imports.get(name);
  if (imp?.file) return finder(files.get(imp.file), imp.name);
  return null;
}

function importers(target, name) {
  const out = [];
  for (const f of files.values()) {
    f.imports ??= parseImports(f);
    for (const [local, imp] of f.imports) if (imp.file === target.abs && imp.name === name) out.push({ f, local });
  }
  return out;
}

function defaultImport(f, name) {
  const re = new RegExp(`\\bimport\\s+${name}\\s+from\\s*(["'])([^"']+)\\1`);
  const m = re.exec(f.code);
  return m ? resolveModule(f.abs, m[2]) : null;
}

function propForwards(f, fnName) {
  if (!f.abs.endsWith(".svelte")) return [];
  const out = [];
  const re = new RegExp(`(?:([A-Za-z_$][\\w$]*)\\s*=\\s*)?\\{\\s*${fnName}\\s*\\}`, "g");
  let m;
  while ((m = re.exec(f.src))) {
    const tagStart = f.src.lastIndexOf("<", m.index);
    const tag = /^<([A-Z][\w$]*)/.exec(f.src.slice(tagStart));
    if (!tag) continue;
    const child = defaultImport(f, tag[1]);
    if (child && files.has(child)) out.push({ f: files.get(child), prop: m[1] ?? fnName });
  }
  return out;
}

function* callSites(f, callee, { member = "none" } = {}) {
  const lead = member === "only" ? "(?<=\\.)" : member === "also" ? "(?<![\\w$])" : "(?<![\\w$.])";
  const re = new RegExp(`${lead}${callee.replace(/\$/g, "\\$")}\\s*(?=[<(])`, "g");
  let m;
  while ((m = re.exec(f.mask))) {
    const before = f.mask.slice(Math.max(0, m.index - 40), m.index);
    if (/\bfunction\s*\*?\s*$/.test(before)) continue;
    let j = m.index + m[0].length;
    if (f.mask[j] === "<") {
      j = skipGeneric(f.mask, j);
      if (j < 0) continue;
      while (/\s/.test(f.mask[j])) j++;
    }
    if (f.mask[j] !== "(") continue;
    const { args, end } = splitArgs(f.mask, j);
    const after = f.mask.slice(end + 1, end + 40);
    if (/^\s*(:[^=;]*)?\{/.test(after) && member !== "only" && !/[=(,:?!&|]\s*$/.test(before) && /(^|[\s;{}])$/.test(before) && !/\breturn\s*$|\bawait\s*$/.test(before)) continue;
    yield { index: m.index, args };
  }
}

function collectTargets(roots, argIndex, kind) {
  const found = [];
  const dynamics = [];
  const seenWrappers = new Set();
  const queue = [];
  for (const r of roots) queue.push({ ...r, argIndex, depth: 0 });
  while (queue.length) {
    const { f, callee, member, argIndex: ai, depth } = queue.shift();
    for (const site of callSites(f, callee, { member })) {
      const arg = site.args[ai];
      if (!arg) continue;
      const res = resolveExpr(f, arg[0], arg[1], depth);
      const line = lineOf(f.src, site.index);
      for (const n of res.names) found.push({ ...n, file: f.path, line });
      if (res.wrapper) {
        const { fn } = res.wrapper;
        const key = `${f.path}#${fn.name}#${fn.index}`;
        if (seenWrappers.has(key)) continue;
        seenWrappers.add(key);
        const next = { argIndex: fn.index, depth: depth + 1 };
        queue.push({ f, callee: fn.name, member: fn.method ? "also" : "none", ...next });
        for (const imp of importers(f, fn.name)) queue.push({ f: imp.f, callee: imp.local, member: "none", ...next });
        for (const p of propForwards(f, fn.name)) queue.push({ f: p.f, callee: p.prop, member: "none", ...next });
      } else if (res.dynamic) {
        dynamics.push({ file: f.path, line, expr: res.dynamic, kind });
      }
    }
  }
  return { found, dynamics, wrappers: [...seenWrappers] };
}

const usesModule = (f, re) => re.test(f.code);

const invokeRoots = [];
const listenRoots = [];
const emitRoots = [];
for (const f of files.values()) {
  if (usesModule(f, /["']@tauri-apps\/api\/core["']/)) invokeRoots.push({ f, callee: "invoke", member: "none" });
  const eventApi = usesModule(f, /["']@tauri-apps\/api\/(event|window|webview|webviewWindow)["']/);
  if (eventApi) {
    listenRoots.push({ f, callee: "listen", member: "none" }, { f, callee: "once", member: "none" });
    emitRoots.push({ f, callee: "emit", member: "none" }, { f, callee: "emitTo", member: "none" });
  }
  listenRoots.push({ f, callee: "listen", member: "only", literalOnly: true }, { f, callee: "once", member: "only", literalOnly: true });
}

const invokes = collectTargets(invokeRoots, 0, "invoke");
const listens = collectTargets(
  listenRoots.filter((r) => !r.literalOnly),
  0,
  "listen",
);
const memberListens = collectTargets(
  listenRoots.filter((r) => r.literalOnly),
  0,
  "listen",
);
listens.found.push(...memberListens.found);
const frontEmits = collectTargets(emitRoots, 0, "emit");
const frontEmitTo = collectTargets(
  emitRoots.filter((r) => r.callee === "emitTo"),
  1,
  "emit",
);
frontEmits.found.push(...frontEmitTo.found);

const libPath = join(ROOT, "src-tauri", "src", "handlers.rs");
const libLexed = lexRust(readFileSync(libPath, "utf8"));
const registered = new Set();
{
  const at = libLexed.mask.indexOf("generate_handler!");
  if (at < 0) {
    console.error("check-ipc: generate_handler! not found in src-tauri/src/handlers.rs");
    process.exit(2);
  }
  const open = libLexed.mask.indexOf("[", at);
  const close = matchBrace(libLexed.mask, open);
  const body = libLexed.code.slice(open + 1, close).replace(/#\[[^\]]*\]/g, "");
  for (const item of body.split(",")) {
    const p = item.trim();
    if (!p) continue;
    registered.add(p.split("::").pop());
  }
}

const rustFiles = walk(join(ROOT, "src-tauri"), [".rs"]).filter((f) => !/[\\/](tests|benches)[\\/]/.test(rel(f)));
const rust = rustFiles.map((abs) => {
  const src = readFileSync(abs, "utf8");
  const lexed = lexRust(src);
  const tests = [];
  const re = /#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{/g;
  let m;
  while ((m = re.exec(lexed.mask))) {
    const open = m.index + m[0].length - 1;
    tests.push([m.index, matchBrace(lexed.mask, open)]);
  }
  return { abs, path: rel(abs), src, ...lexed, inTest: (i) => tests.some(([a, b]) => i >= a && i <= b) };
});

const rustConsts = new Map();
const constDecls = [];
for (const r of rust) {
  const re = /\b(?:const|static)\s+([A-Z_][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"\\]*)"/g;
  let m;
  while ((m = re.exec(r.code))) {
    if (r.inTest(m.index)) continue;
    if (!rustConsts.has(m[1])) rustConsts.set(m[1], new Set());
    rustConsts.get(m[1]).add(m[2]);
    constDecls.push({ name: m[1], value: m[2], file: r.path, index: m.index });
  }
}

const rustEmits = [];
function rustArgNames(r, a, b) {
  let text = r.code.slice(a, b).trim();
  text = text.replace(/^&\*?/, "").replace(/\.(to_string|to_owned|into|as_str|as_ref)\(\)$/, "").trim();
  const lit = /^"([^"\\]*)"$/.exec(text);
  if (lit) return [{ exact: lit[1] }];
  const fmt = /^format!\s*\(\s*"([^"]*)"/.exec(text);
  if (fmt) {
    const k = fmt[1].indexOf("{");
    return [k < 0 ? { exact: fmt[1] } : { prefix: fmt[1].slice(0, k) }];
  }
  const path = /^(?:[A-Za-z_][\w]*::)*([A-Z_][A-Z0-9_]*)$/.exec(text);
  if (path && rustConsts.has(path[1])) return [...rustConsts.get(path[1])].map((exact) => ({ exact }));
  return [];
}
for (const r of rust) {
  const re = /(?<![\w])([A-Za-z_]\w*)\s*(?:::<[^>()]*>)?\s*\(/g;
  let m;
  while ((m = re.exec(r.mask))) {
    if (!/emit/i.test(m[1])) continue;
    if (r.inTest(m.index)) continue;
    if (/\bfn\s+$/.test(r.mask.slice(Math.max(0, m.index - 10), m.index))) continue;
    const open = r.mask.indexOf("(", m.index + m[1].length);
    const { args } = splitArgs(r.mask, open);
    for (const [a, b] of args.slice(0, 3)) {
      for (const n of rustArgNames(r, a, b)) rustEmits.push({ ...n, file: r.path, line: lineOf(r.src, m.index) });
    }
  }
}

{
  const byName = new Map();
  for (const d of constDecls) byName.set(d.name, [...(byName.get(d.name) ?? []), d]);
  const names = [...byName.keys()];
  if (names.length) {
    const re = new RegExp(`(?<![\\w])(${names.join("|")})(?![\\w])`, "g");
    for (const r of rust) {
      let m;
      while ((m = re.exec(r.mask))) {
        if (r.inTest(m.index)) continue;
        if (byName.get(m[1]).some((d) => d.file === r.path && d.index <= m.index && m.index < d.index + 200 && /^(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+$/.test(r.mask.slice(d.index, m.index).replace(/^.*?(?=(pub|const|static))/s, "")))) continue;
        const before = r.mask.slice(Math.max(0, m.index - 80), m.index);
        if (/\b(listen|once|listen_any|unlisten)\s*\([^;]*$/.test(before)) continue;
        for (const exact of rustConsts.get(m[1])) rustEmits.push({ exact, file: r.path, line: lineOf(r.src, m.index), indirect: true });
      }
    }
  }
}

const allowPath = join(ROOT, "scripts", "ipc-allowlist.json");
const allow = existsSync(allowPath) ? JSON.parse(readFileSync(allowPath, "utf8")) : {};
const allowErrors = [];
for (const [section, entries] of Object.entries(allow)) {
  if (section.startsWith("$")) continue;
  for (const e of entries) if (!e.reason || !String(e.reason).trim()) allowErrors.push(`${section}: entry without a reason: ${JSON.stringify(e)}`);
}
const allowedCommands = new Set((allow.commands ?? []).map((e) => e.name));
const allowedEvents = new Set((allow.events ?? []).map((e) => e.name));
const allowedDynamic = (allow.dynamic ?? []).map((e) => ({ ...e, used: false }));
const isAllowedDynamic = (d) => {
  const hit = allowedDynamic.find((e) => e.file === d.file && e.expr === d.expr && (!e.kind || e.kind === d.kind));
  if (hit) hit.used = true;
  return !!hit;
};

const BUILTIN_EVENT = /^tauri:\/\//;
const errors = [];

const invokedNames = new Set();
for (const t of invokes.found) {
  if (t.prefix !== undefined) {
    const any = [...registered].some((c) => c.startsWith(t.prefix));
    if (!any && !t.prefix.startsWith("plugin:")) errors.push(`${t.file}:${t.line}: invoke of \`${t.prefix}...\` matches no registered command`);
    for (const c of registered) if (c.startsWith(t.prefix)) invokedNames.add(c);
    continue;
  }
  invokedNames.add(t.exact);
  if (t.exact.startsWith("plugin:")) continue;
  if (!registered.has(t.exact) && !allowedCommands.has(t.exact)) {
    errors.push(`${t.file}:${t.line}: invoke("${t.exact}") is not registered in generate_handler! (src-tauri/src/handlers.rs)`);
  }
}

const emitted = [...rustEmits, ...frontEmits.found];
const emitsMatch = (t) =>
  emitted.some((e) => {
    if (t.exact !== undefined) return e.exact === t.exact || (e.prefix !== undefined && t.exact.startsWith(e.prefix));
    return (e.exact !== undefined && e.exact.startsWith(t.prefix)) || (e.prefix !== undefined && (e.prefix.startsWith(t.prefix) || t.prefix.startsWith(e.prefix)));
  });
for (const t of listens.found) {
  const name = t.exact ?? `${t.prefix}*`;
  if (t.exact !== undefined && BUILTIN_EVENT.test(t.exact)) continue;
  if (allowedEvents.has(name)) continue;
  if (!emitsMatch(t)) errors.push(`${t.file}:${t.line}: listen("${name}") has no emitter (Rust emit* call or frontend emit)`);
}

for (const d of [...invokes.dynamics, ...listens.dynamics]) {
  if (isAllowedDynamic(d)) continue;
  errors.push(`${d.file}:${d.line}: ${d.kind} target \`${d.expr}\` could not be resolved; resolve it statically or add it to scripts/ipc-allowlist.json ("dynamic") with a reason`);
}
for (const e of allowedDynamic) if (!e.used) errors.push(`scripts/ipc-allowlist.json: stale "dynamic" entry ${e.file} \`${e.expr}\` (no such call site any more)`);
for (const name of allowedCommands) if (registered.has(name)) errors.push(`scripts/ipc-allowlist.json: "commands" entry ${name} is registered now; remove it`);
errors.push(...allowErrors.map((e) => `scripts/ipc-allowlist.json: ${e}`));

const neverInvoked = [...registered].filter((c) => !invokedNames.has(c)).sort();

const result = {
  ok: errors.length === 0,
  registered: registered.size,
  invokeSites: invokes.found.length,
  listenSites: listens.found.length,
  emittedNames: new Set(emitted.map((e) => e.exact ?? `${e.prefix}*`)).size,
  wrappers: [...invokes.wrappers, ...listens.wrappers],
  errors,
  neverInvoked,
};

if (asJson) {
  console.log(JSON.stringify(result, null, 2));
} else {
  console.log(
    `check-ipc: ${registered.size} registered commands, ${invokes.found.length} invoke targets, ${listens.found.length} listen targets, ${result.emittedNames} emitted event names, ${result.wrappers.length} wrappers followed`,
  );
  if (verbose) {
    for (const t of invokes.found) console.log(`  invoke  ${t.exact ?? `${t.prefix}*`}  (${t.file}:${t.line})`);
    for (const t of listens.found) console.log(`  listen  ${t.exact ?? `${t.prefix}*`}  (${t.file}:${t.line})`);
    for (const w of result.wrappers) console.log(`  wrapper ${w}`);
  }
  if (neverInvoked.length) {
    console.log(`\ninfo: ${neverInvoked.length} registered commands the frontend never invokes (CLI, MCP or dead code; not a failure):`);
    for (const c of neverInvoked) console.log(`  - ${c}`);
  }
  if (errors.length) {
    console.log(`\nFAIL: ${errors.length} IPC contract violation(s):`);
    for (const e of errors) console.log(`  - ${e}`);
  } else {
    console.log("\nOK: every invoke is registered and every listened event has an emitter.");
  }
}
process.exit(errors.length ? 1 : 0);
