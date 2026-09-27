// Mini parser de YAML para frontmatter (subconjunto): mapas e listas por
// indentação, escalares simples/aspas, blocos `|`/`>`, fluxo `[..]`/`{..}` e
// escalares simples continuados em várias linhas. Sem dependência externa.
// O port em Rust fica em src-tauri/omniget-core/src/core/catalog/frontmatter.rs.

/** Separa `---\n...\n---` do corpo. Devolve { yaml, body } ou null. */
export function splitFrontmatter(text) {
  const t = text.replace(/^﻿/, '');
  if (!/^---[ \t]*\r?\n/.test(t)) return null;
  const rest = t.slice(t.indexOf('\n') + 1);
  const m = /(^|\r?\n)---[ \t]*(\r?\n|$)/.exec(rest);
  if (!m) return null;
  const yaml = rest.slice(0, m.index);
  const body = rest.slice(m.index + m[0].length);
  return { yaml, body };
}

function indentOf(line) {
  let i = 0;
  while (i < line.length && line[i] === ' ') i++;
  return i;
}

function isBlank(line) {
  const s = line.trim();
  return s === '' || s.startsWith('#');
}

function stripComment(s) {
  // Remove ` # comentário` fora de aspas.
  let q = null;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (q) {
      if (c === q) q = null;
    } else if (c === '"' || c === "'") {
      if (i === 0 || /\s|[\[{,:]/.test(s[i - 1])) q = c;
    } else if (c === '#' && (i === 0 || /\s/.test(s[i - 1]))) {
      return s.slice(0, i).trimEnd();
    }
  }
  return s;
}

function unescapeDouble(s) {
  return s.replace(/\\(u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|.)/g, (_, e) => {
    switch (e[0]) {
      case 'n': return '\n';
      case 't': return '\t';
      case 'r': return '\r';
      case '0': return '\0';
      case '"': return '"';
      case '\\': return '\\';
      case '/': return '/';
      case 'u': return String.fromCharCode(parseInt(e.slice(1), 16));
      case 'x': return String.fromCharCode(parseInt(e.slice(1), 16));
      default: return e;
    }
  });
}

function plainScalar(s) {
  const v = s.trim();
  if (v === '' || v === '~' || v === 'null' || v === 'Null' || v === 'NULL') return null;
  if (v === 'true' || v === 'True' || v === 'TRUE') return true;
  if (v === 'false' || v === 'False' || v === 'FALSE') return false;
  if (/^-?(0|[1-9][0-9]{0,14})$/.test(v)) return Number(v);
  return v;
}

/** Parser de fluxo: `[a, "b", {c: d}]`. */
function parseFlow(src) {
  let i = 0;
  function ws() { while (i < src.length && /\s/.test(src[i])) i++; }
  function value(stop) {
    ws();
    const c = src[i];
    if (c === '[') {
      i++;
      const arr = [];
      ws();
      if (src[i] === ']') { i++; return arr; }
      while (i < src.length) {
        arr.push(value(',]'));
        ws();
        if (src[i] === ',') { i++; ws(); if (src[i] === ']') { i++; return arr; } continue; }
        if (src[i] === ']') { i++; return arr; }
        break;
      }
      return arr;
    }
    if (c === '{') {
      i++;
      const obj = {};
      ws();
      if (src[i] === '}') { i++; return obj; }
      while (i < src.length) {
        const k = value(':,}');
        ws();
        let v = null;
        if (src[i] === ':') { i++; v = value(',}'); }
        obj[String(k)] = v;
        ws();
        if (src[i] === ',') { i++; continue; }
        if (src[i] === '}') { i++; return obj; }
        break;
      }
      return obj;
    }
    if (c === '"' || c === "'") {
      i++;
      let out = '';
      while (i < src.length) {
        if (c === '"' && src[i] === '\\') { out += src.slice(i, i + 2); i += 2; continue; }
        if (src[i] === c) {
          if (c === "'" && src[i + 1] === "'") { out += "'"; i += 2; continue; }
          i++;
          break;
        }
        out += src[i++];
      }
      return c === '"' ? unescapeDouble(out) : out;
    }
    let start = i;
    while (i < src.length && !stop.includes(src[i])) i++;
    return plainScalar(src.slice(start, i));
  }
  const v = value('');
  return v;
}

function scalarFrom(text) {
  const s = text.trim();
  if (s.startsWith('"')) {
    const end = findClose(s, '"');
    if (end > 0) return unescapeDouble(s.slice(1, end));
    return unescapeDouble(s.slice(1));
  }
  if (s.startsWith("'")) {
    const end = findClose(s, "'");
    if (end > 0) return s.slice(1, end).replace(/''/g, "'");
    return s.slice(1).replace(/''/g, "'");
  }
  const c = stripComment(s);
  if (c.startsWith('[') || c.startsWith('{')) {
    try { return parseFlow(c); } catch { return c; }
  }
  return plainScalar(c);
}

function findClose(s, q) {
  for (let i = 1; i < s.length; i++) {
    if (q === '"' && s[i] === '\\') { i++; continue; }
    if (s[i] === q) {
      if (q === "'" && s[i + 1] === "'") { i++; continue; }
      return i;
    }
  }
  return -1;
}

const KEY_RE = /^("(?:[^"\\]|\\.)*"|'(?:[^']|'')*'|[^\s#'"\-?:,\[\]{}][^#]*?|-[^\s][^#]*?)\s*:(?:\s+(.*)|\s*)$/;

function keyText(k) {
  if (k.startsWith('"')) return unescapeDouble(k.slice(1, -1));
  if (k.startsWith("'")) return k.slice(1, -1).replace(/''/g, "'");
  return k.trim();
}

class P {
  constructor(text) {
    this.lines = text.replace(/\t/g, '  ').split(/\r?\n/);
    this.i = 0;
  }
  skip() {
    while (this.i < this.lines.length && isBlank(this.lines[this.i])) this.i++;
  }
  node(minIndent) {
    this.skip();
    if (this.i >= this.lines.length) return null;
    const line = this.lines[this.i];
    const ind = indentOf(line);
    if (ind < minIndent) return null;
    const s = line.slice(ind);
    if (s === '-' || s.startsWith('- ')) return this.list(ind);
    if (KEY_RE.test(s)) return this.map(ind);
    // Escalar solto (texto multi-linha).
    const parts = [];
    while (this.i < this.lines.length && (isBlank(this.lines[this.i]) || indentOf(this.lines[this.i]) >= ind)) {
      const t = this.lines[this.i].trim();
      if (t) parts.push(t);
      this.i++;
    }
    return scalarFrom(parts.join(' '));
  }
  blockScalar(header, parentIndent) {
    const fold = header.startsWith('>');
    const keep = header.includes('+');
    const strip = header.includes('-');
    const raw = [];
    let blockIndent = null;
    while (this.i < this.lines.length) {
      const l = this.lines[this.i];
      if (l.trim() === '') { raw.push(''); this.i++; continue; }
      const ind = indentOf(l);
      if (ind <= parentIndent) break;
      if (blockIndent === null) blockIndent = ind;
      if (ind < blockIndent) break;
      raw.push(l.slice(blockIndent));
      this.i++;
    }
    let text;
    if (fold) {
      text = '';
      for (let k = 0; k < raw.length; k++) {
        const l = raw[k];
        if (l === '') { text += '\n'; continue; }
        if (text && !text.endsWith('\n') && !/^\s/.test(l)) text += ' ';
        else if (text && !text.endsWith('\n')) text += '\n';
        text += l;
      }
    } else {
      text = raw.join('\n');
    }
    if (strip) return text.replace(/\n+$/, '');
    if (keep) return text + '\n';
    return text.replace(/\n+$/, '') + '\n';
  }
  inlineValue(rest, ownIndent) {
    // valor na mesma linha da chave ou do traço
    const r = rest.trim();
    if (r === '' || r.startsWith('#')) {
      this.skip();
      if (this.i < this.lines.length) {
        const nl = this.lines[this.i];
        const ni = indentOf(nl);
        const ns = nl.slice(ni);
        if (ni > ownIndent) return this.node(ni);
        if (ni === ownIndent && (ns === '-' || ns.startsWith('- '))) return this.list(ni);
      }
      return null;
    }
    if (/^[|>][+-]?[0-9]?\s*(#.*)?$/.test(r)) return this.blockScalar(r, ownIndent);
    // aspas abertas que continuam em outras linhas
    if ((r[0] === '"' || r[0] === "'") && findClose(r, r[0]) < 0) {
      let acc = r;
      while (this.i < this.lines.length) {
        const l = this.lines[this.i++].trim();
        acc += ' ' + l;
        if (findClose(acc, r[0]) > 0) break;
      }
      return scalarFrom(acc);
    }
    if ((r[0] === '[' || r[0] === '{')) {
      let acc = stripComment(r);
      const open = r[0] === '[' ? ']' : '}';
      let guard = 0;
      while (!balanced(acc) && this.i < this.lines.length && guard++ < 500) {
        acc += ' ' + stripComment(this.lines[this.i++].trim());
      }
      void open;
      try { return parseFlow(acc); } catch { return acc; }
    }
    // escalar simples continuado em linhas mais indentadas
    const parts = [r];
    while (this.i < this.lines.length) {
      const l = this.lines[this.i];
      if (l.trim() === '') {
        // linha em branco dentro de escalar simples: só continua se a próxima ainda é continuação
        let j = this.i + 1;
        while (j < this.lines.length && this.lines[j].trim() === '') j++;
        if (j < this.lines.length && indentOf(this.lines[j]) > ownIndent && !KEY_RE.test(this.lines[j].trim())) {
          parts.push('\n');
          this.i = j;
          continue;
        }
        break;
      }
      const ind = indentOf(l);
      if (ind <= ownIndent) break;
      parts.push(l.trim());
      this.i++;
    }
    if (parts.length === 1) return scalarFrom(r);
    const joined = parts.join(' ').replace(/ ?\n ?/g, '\n');
    const v = scalarFrom(joined);
    return v;
  }
  map(ind) {
    const obj = {};
    while (this.i < this.lines.length) {
      this.skip();
      if (this.i >= this.lines.length) break;
      const line = this.lines[this.i];
      const li = indentOf(line);
      if (li < ind) break;
      if (li > ind) { this.i++; continue; }
      const s = line.slice(li);
      if (s === '-' || s.startsWith('- ')) break;
      const m = KEY_RE.exec(s);
      if (!m) { this.i++; continue; }
      this.i++;
      obj[keyText(m[1])] = this.inlineValue(m[2] ?? '', ind);
    }
    return obj;
  }
  list(ind) {
    const arr = [];
    while (this.i < this.lines.length) {
      this.skip();
      if (this.i >= this.lines.length) break;
      const line = this.lines[this.i];
      const li = indentOf(line);
      if (li !== ind) break;
      const s = line.slice(li);
      if (!(s === '-' || s.startsWith('- '))) break;
      const rest = s === '-' ? '' : s.slice(2);
      const restTrim = rest.trimStart();
      const col = li + 2 + (rest.length - restTrim.length);
      if (restTrim !== '' && KEY_RE.test(restTrim) && !restTrim.startsWith('"') && !restTrim.startsWith("'")) {
        // item-mapa: reescreve a linha como chave na coluna do texto
        this.lines[this.i] = ' '.repeat(col) + restTrim;
        arr.push(this.map(col));
      } else {
        this.i++;
        arr.push(this.inlineValue(restTrim, li));
      }
    }
    return arr;
  }
}

function balanced(s) {
  let depth = 0;
  let q = null;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (q) {
      if (c === '\\' && q === '"') { i++; continue; }
      if (c === q) q = null;
      continue;
    }
    if (c === '"' || c === "'") q = c;
    else if (c === '[' || c === '{') depth++;
    else if (c === ']' || c === '}') depth--;
  }
  return depth <= 0;
}

/** YAML → valor JS. Nunca lança: em erro devolve pares `chave: valor` linha a linha. */
export function parseYaml(text) {
  try {
    const p = new P(text);
    const v = p.node(0);
    if (v && typeof v === 'object') return v;
    return v === null ? {} : { _value: v };
  } catch {
    const obj = {};
    for (const line of text.split(/\r?\n/)) {
      const m = /^([A-Za-z0-9_-]+)\s*:\s*(.*)$/.exec(line);
      if (m) obj[m[1]] = m[2].trim();
    }
    return obj;
  }
}
