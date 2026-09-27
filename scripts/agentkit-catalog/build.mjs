#!/usr/bin/env node
// Gerador do índice do Catálogo da Central (agentkit).
//
// Lê checkouts pinados das fontes (davila7/claude-code-templates + repos de
// skills oficiais) e escreve static/agentkit/catalog-index.json.gz (o que o
// app embute) e catalog-meta.json. O JSON puro vai para
// scripts/agentkit-catalog/out/ (só inspeção, fora do git).
// O conteúdo NÃO é copiado: o app baixa cada arquivo de
// raw.githubusercontent.com/<repo>/<commit>/<path> e confere o sha256 do índice.
//
// Uso:
//   node scripts/agentkit-catalog/build.mjs [--latest] [--src <dir>] [--commit <sha>]
//        [--work <dir>] [--out <dir>] [--no-network] [--only-cct]
//
//   --latest   resolve o commit mais novo de cada fonte (git ls-remote) e
//              regrava pins.json; sem ele usa os commits de pins.json.
//   --src      checkout já pronto do claude-code-templates.
//   --work     pasta dos clones rasos (padrão: <tmp>/omniget-agentkit-catalog).
//
// Sem dependências npm: só builtins `node:`. A lógica de descoberta por
// caminho é espelhada em Rust em omniget-core/src/core/catalog/scan.rs.

import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';
import { parseYaml, splitFrontmatter } from './yaml.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');
const PINS_FILE = path.join(HERE, 'pins.json');
const CCT_REPO = 'davila7/claude-code-templates';
const CCT_URL = `https://github.com/${CCT_REPO}`;
const DEFAULT_COMMIT = '590d437baf1e53d934a976bd8a06af54607ed7e1';
const DESC_MAX = 300;

/**
 * Repos de skills com licença por item. `roots`: pastas varridas atrás de
 * SKILL.md (`category` fixa ou, sem ela, a primeira pasta abaixo da raiz).
 * `license`: licença do repo, usada quando a skill não declara nenhuma.
 * `excludeNames`: skills fora do índice por licença (source-available).
 */
const SKILL_SOURCES = [
  {
    id: 'anthropics/skills',
    name: 'Anthropic Agent Skills',
    repo: 'anthropics/skills',
    roots: [{ dir: 'skills', category: 'anthropic' }],
    // README: "Many skills in this repo are open source (Apache 2.0)" menos os 4 de documento
    license: 'Apache-2.0',
    origin: 'claude',
    author: 'Anthropic',
    excludeNames: { docx: 'source_available', pdf: 'source_available', pptx: 'source_available', xlsx: 'source_available' },
    note: 'docx/pdf/pptx/xlsx are source-available (not open source) and are excluded.',
  },
  {
    id: 'openai/skills',
    name: 'OpenAI Skills (Codex)',
    repo: 'openai/skills',
    roots: [
      { dir: 'skills/.curated', category: 'curated' },
      { dir: 'skills/.system', category: 'system' },
    ],
    license: null,
    origin: 'codex',
    author: 'OpenAI',
    codexMetadata: true,
    note: 'License per skill (LICENSE.txt). Figma skills are under the Figma Developer Terms and are excluded.',
  },
  {
    id: 'google/skills',
    name: 'Google Agent Skills',
    repo: 'google/skills',
    // `plugins/` repete 5 skills de `skills/` dentro de um plugin; fica de fora
    roots: [{ dir: 'skills' }],
    license: 'Apache-2.0',
    origin: 'gemini',
    author: 'Google',
  },
  {
    id: 'obra/superpowers',
    name: 'Superpowers',
    repo: 'obra/superpowers',
    roots: [{ dir: 'skills', category: 'workflow' }],
    license: 'MIT',
    origin: 'claude',
    author: 'obra',
  },
  {
    id: 'K-Dense-AI/scientific-agent-skills',
    name: 'Scientific Agent Skills (K-Dense)',
    repo: 'K-Dense-AI/scientific-agent-skills',
    roots: [{ dir: 'skills', category: 'scientific' }],
    license: 'MIT',
    origin: 'claude',
    author: 'K-Dense Inc.',
    excludeNames: { docx: 'anthropic_proprietary_document_skill', pdf: 'anthropic_proprietary_document_skill', pptx: 'anthropic_proprietary_document_skill', xlsx: 'anthropic_proprietary_document_skill' },
    // ex-K-Dense-AI/claude-scientific-skills (o GitHub redireciona)
    formerly: 'K-Dense-AI/claude-scientific-skills',
  },
];

/** Fontes avaliadas e deixadas de fora (vão no meta, com o motivo). */
const SKIPPED_SOURCES = [
  {
    repo: 'OpenRouterTeam/skills',
    reason: 'already_pinned_elsewhere',
    detail: 'Pinned and installed by omniget-core/src/core/skills/catalog.rs; the repository declares no license.',
  },
  {
    repo: 'hesreallyhim/awesome-claude-code',
    reason: 'links_only',
    detail: 'An awesome list: resources are links in a CSV/README, no component files to index (license NOASSERTION).',
  },
];

// ---------------------------------------------------------------- argumentos
function args() {
  const a = process.argv.slice(2);
  const out = {
    src: null,
    commit: null,
    out: path.join(ROOT, 'static', 'agentkit'),
    inspect: path.join(HERE, 'out'),
    work: path.join(os.tmpdir(), 'omniget-agentkit-catalog'),
    network: true,
    latest: false,
    onlyCct: false,
  };
  for (let i = 0; i < a.length; i++) {
    const k = a[i];
    if (k === '--src') out.src = path.resolve(a[++i]);
    else if (k === '--commit') out.commit = a[++i];
    else if (k === '--out') out.out = path.resolve(a[++i]);
    else if (k === '--work') out.work = path.resolve(a[++i]);
    else if (k === '--no-network') out.network = false;
    else if (k === '--latest') out.latest = true;
    else if (k === '--only-cct') out.onlyCct = true;
    else if (k === '-h' || k === '--help') {
      console.log('node scripts/agentkit-catalog/build.mjs [--latest] [--src dir] [--commit sha] [--work dir] [--out dir] [--no-network] [--only-cct]');
      process.exit(0);
    } else throw new Error(`argumento desconhecido: ${k}`);
  }
  return out;
}

function git(cwd, ...a) {
  return execFileSync('git', a, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 64 << 20 }).trim();
}

function readPins() {
  try {
    return JSON.parse(fs.readFileSync(PINS_FILE, 'utf8'));
  } catch {
    return {};
  }
}

/** Clone raso de `repo` no `commit` (reaproveita a pasta se o HEAD já bate). */
function checkout(repo, commit, dir) {
  let head = null;
  try {
    head = git(dir, 'rev-parse', 'HEAD');
  } catch {
    /* sem clone */
  }
  if (head !== commit) {
    console.error(`[catalog] clonando ${repo}@${commit.slice(0, 7)} em ${dir}`);
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    git(dir, 'init', '-q');
    git(dir, 'remote', 'add', 'origin', `https://github.com/${repo}.git`);
    git(dir, 'fetch', '-q', '--depth', '1', 'origin', commit);
    git(dir, 'checkout', '-q', 'FETCH_HEAD');
  }
  let date = null;
  try {
    date = git(dir, 'log', '-1', '--format=%cI');
  } catch {
    /* ok */
  }
  return { commit, date: date || new Date().toISOString() };
}

function ensureCheckout(src, commit) {
  if (!fs.existsSync(path.join(src, 'cli-tool', 'components'))) {
    return checkout(CCT_REPO, commit || DEFAULT_COMMIT, src);
  }
  let head = null;
  let date = null;
  try {
    head = git(src, 'rev-parse', 'HEAD');
    date = git(src, 'log', '-1', '--format=%cI');
  } catch {
    /* pasta sem .git: confia no --commit */
  }
  if (commit && head && head !== commit) return checkout(CCT_REPO, commit, src);
  const c = commit || head;
  if (!c) throw new Error('commit desconhecido: passe --commit');
  return { commit: c, date: date || new Date().toISOString() };
}

// ---------------------------------------------------------------- utilidades
const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');
const posix = (p) => p.split(path.sep).join('/');
const SKIP_NAMES = new Set(['.git', '.DS_Store', 'node_modules', '__pycache__', 'Thumbs.db']);

function walkFiles(dir, { skipDirs = () => false, skipDot = false } = {}) {
  const out = [];
  (function rec(d) {
    for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (SKIP_NAMES.has(e.name)) continue;
      const full = path.join(d, e.name);
      if (skipDot && e.name.startsWith('.') && e.name !== '.claude-plugin') continue;
      if (e.isDirectory()) {
        if (skipDirs(full)) continue;
        rec(full);
      } else if (e.isFile()) out.push(full);
    }
  })(dir);
  return out;
}

function fileEntry(base, full) {
  const buf = fs.readFileSync(full);
  return { path: posix(path.relative(base, full)), sha256: sha256(buf), size: buf.length };
}

function readText(p) {
  return fs.readFileSync(p, 'utf8');
}

function readJson(p) {
  const t = readText(p);
  try {
    return JSON.parse(t);
  } catch {
    // vírgula sobrando
    return JSON.parse(t.replace(/,(\s*[}\]])/g, '$1'));
  }
}

function listDirs(d) {
  if (!fs.existsSync(d)) return [];
  return fs
    .readdirSync(d, { withFileTypes: true })
    .filter((e) => e.isDirectory() && !SKIP_NAMES.has(e.name) && !e.name.startsWith('.'))
    .map((e) => e.name)
    .sort();
}

function listFiles(d) {
  if (!fs.existsSync(d)) return [];
  return fs
    .readdirSync(d, { withFileTypes: true })
    .filter((e) => e.isFile() && !SKIP_NAMES.has(e.name))
    .map((e) => e.name)
    .sort();
}

// ---------------------------------------------------------------- normalização
/** Descrição curta: sem <example>/<commentary>, espaço colapsado, ≤ 300 chars. */
export function shortDescription(raw, norm) {
  if (raw === null || raw === undefined) return '';
  let s = typeof raw === 'string' ? raw : JSON.stringify(raw);
  const before = s;
  s = s.replace(/\\n/g, '\n');
  s = s.replace(/<example>[\s\S]*?<\/example>/gi, ' ');
  s = s.replace(/<commentary>[\s\S]*?<\/commentary>/gi, ' ');
  s = s.replace(/<example>[\s\S]*$/i, ' ');
  s = s.replace(/<\/?(example|commentary|context|user|assistant)>/gi, ' ');
  if (s !== before.replace(/\\n/g, '\n') && norm) norm.add('stripped_example_blocks');
  s = s.replace(/\s+/g, ' ').trim();
  s = s.replace(/\s*(Examples?( include)?|For example|e\.g\.|Specifically|Such as|Including|Use cases?)\s*:?\s*$/i, '').trim();
  const cps = [...s];
  if (cps.length > DESC_MAX) {
    const cut = cps.slice(0, DESC_MAX - 1).join('');
    const sp = cut.lastIndexOf(' ');
    const spCps = sp < 0 ? -1 : [...cut.slice(0, sp)].length;
    s = (spCps > DESC_MAX * 0.6 ? cut.slice(0, sp) : cut).replace(/[\s,;:.-]+$/, '') + '…';
    if (norm) norm.add('truncated_description');
  }
  return s;
}

/** Descrição longa sem blocos <example>/<commentary> (vai no frontmatter do índice). */
export function cleanDescription(raw) {
  if (typeof raw !== 'string') return raw;
  let s = raw.replace(/<example>[\s\S]*?<\/example>/gi, ' ').replace(/<commentary>[\s\S]*?<\/commentary>/gi, ' ');
  s = s.replace(/<example>[\s\S]*$/i, ' ').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
  return s.replace(/\s*(Examples?( include)?|For example|e\.g\.|Specifically|Such as|Including|Use cases?)\s*:?\s*$/i, '').trim();
}

/** Primeiro parágrafo útil de um corpo Markdown (para sintetizar descrição). */
export function descriptionFromBody(body) {
  const lines = body.split(/\r?\n/);
  let heading = null;
  const para = [];
  let inCode = false;
  for (const l of lines) {
    const t = l.trim();
    if (t.startsWith('```')) { inCode = !inCode; continue; }
    if (inCode) continue;
    if (!t) { if (para.length) break; continue; }
    if (t.startsWith('#')) { if (!heading) heading = t.replace(/^#+\s*/, ''); if (para.length) break; continue; }
    if (/^(---|\*\*\*|<!--|\||>)/.test(t)) { if (para.length) break; continue; }
    para.push(t.replace(/^[-*]\s+/, ''));
    if (para.join(' ').length > 400) break;
  }
  let s = para.join(' ') || heading || '';
  s = s.replace(/\*\*|__|`/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
  return s;
}

function humanize(name) {
  return name.replace(/[-_]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

const CLAUDE_MODEL_ALIASES = new Set(['sonnet', 'opus', 'haiku', 'inherit', 'default', 'opusplan']);
/** Modelo válido para Claude Code: alias ou id datado `claude-...-YYYYMMDD`. */
export function validClaudeModel(m) {
  if (typeof m !== 'string') return false;
  const v = m.trim();
  return CLAUDE_MODEL_ALIASES.has(v) || /^claude-[a-z0-9.-]+-\d{8}$/.test(v);
}

const COPILOT_TOOLS = new Set([
  'codebase', 'search', 'editFiles', 'runCommands', 'runTests', 'problems', 'githubRepo', 'fetch',
  'findTestFiles', 'usages', 'testFailure', 'vscodeAPI', 'openSimpleBrowser', 'changes', 'extensions',
  'searchResults', 'terminalLastCommand', 'terminalSelection', 'runTasks', 'runNotebooks', 'new',
  'terminalCommand', 'filesystem', 'database', 'github', 'websearch', 'think', 'todos',
]);
const CLAUDE_TOOLS = new Set([
  'Read', 'Write', 'Edit', 'MultiEdit', 'Bash', 'Grep', 'Glob', 'LS', 'WebSearch', 'WebFetch', 'TodoWrite',
  'Task', 'Agent', 'NotebookEdit', 'NotebookRead', 'BashOutput', 'KillShell', 'Skill', 'AskUserQuestion',
]);
function toolList(tools) {
  let list = [];
  if (Array.isArray(tools)) list = tools.map(String);
  else if (typeof tools === 'string') list = tools.split(',');
  return list.map((t) => t.trim().replace(/^['"]|['"]$/g, '')).filter(Boolean);
}
const isVsCodeTool = (t) =>
  COPILOT_TOOLS.has(t) || /^[a-z][\w.-]*\/[\w*]/.test(t) || /^(azure_|mssql_|pgsql_)|^microsoft\.docs\.mcp$/.test(t) ||
  ['read', 'edit', 'shell', 'execute', 'web', 'agent', 'todo', 'vscode', 'runSubagent'].includes(t);
/**
 * Classifica o `tools` de um agente: 'copilot' (chatmode do Copilot com ids do
 * VS Code), 'mixed' (ferramentas do Claude + ids estranhos), ou 'claude'.
 */
export function toolsOrigin(tools, model) {
  const list = toolList(tools);
  const hasClaude = list.some((t) => CLAUDE_TOOLS.has(t) || /^(Bash|Read|Write|Edit|WebFetch)\(/.test(t) || t.startsWith('mcp__') || t === '*');
  const foreign = list.filter((t) => !CLAUDE_TOOLS.has(t) && !t.startsWith('mcp__') && t !== '*' && !/^[A-Z][A-Za-z]+(\(.*\))?$/.test(t));
  if (typeof model === 'string' && /\(copilot\)/i.test(model)) return 'copilot';
  if (!hasClaude && list.some(isVsCodeTool)) return 'copilot';
  if (hasClaude && foreign.length) return 'mixed';
  return 'claude';
}

// licenças --------------------------------------------------------------------
const FIGMA_TERMS = 'LicenseRef-Figma-Developer-Terms';
/** Licenças que não permitem redistribuir: item fica fora do índice. */
export function nonRedistributable(license) {
  if (license === 'Proprietary') return 'proprietary_license';
  if (license === FIGMA_TERMS) return 'vendor_terms_not_open_source';
  return null;
}
export function normalizeLicense(s) {
  if (s === null || s === undefined) return null;
  if (typeof s === 'object') s = s.type || s.name || s.spdx || null;
  if (!s || typeof s !== 'string') return null;
  let v = s.trim();
  if (/proprietary/i.test(v)) return 'Proprietary';
  if (/complete terms in|see license|license\.(txt|md)/i.test(v) && !/\b(MIT|Apache)\b/.test(v)) return null;
  if (/^(unknown|none|n\/a|unlicensed)$/i.test(v)) return null;
  // "This skill is provided under the MIT License. …"
  const under = /\bunder the (MIT|Apache[- ]2\.0|BSD[- ]3[- ]Clause) License\b/i.exec(v);
  if (under && v.length > 40) v = under[1];
  if (/^https?:\/\/creativecommons\.org\/licenses\/by\/4\.0\/?$/i.test(v)) return 'CC-BY-4.0';
  if (/^https?:\/\/creativecommons\.org\/licenses\/by-sa\/4\.0\/?$/i.test(v)) return 'CC-BY-SA-4.0';
  // "MIT license", "BSD-3-Clause license", "GPLv3 license"
  v = v.replace(/\s+licen[cs]e$/i, '').replace(/^GPLv(\d)$/i, 'GPL-$1.0');
  const map = [
    [/^mit( license)?$/i, 'MIT'],
    [/^apache[- ]?(license)?[ ,-]*(v(ersion)?)? ?2(\.0)?$/i, 'Apache-2.0'],
    [/^cc[- ]?by[- ]?4(\.0)?$/i, 'CC-BY-4.0'],
    [/^cc[- ]?by[- ]?sa[- ]?4(\.0)?$/i, 'CC-BY-SA-4.0'],
    [/^cc0([- ]?1(\.0)?)?$/i, 'CC0-1.0'],
    [/^bsd[- ]?3([- ]?clause)?$/i, 'BSD-3-Clause'],
    [/^3[- ]?clause[- ]bsd$/i, 'BSD-3-Clause'],
    [/^cc[- ]?by[- ]?nc[- ]?sa[- ]?4(\.0)?$/i, 'CC-BY-NC-SA-4.0'],
    [/^cc[- ]?by[- ]?nc[- ]?4(\.0)?$/i, 'CC-BY-NC-4.0'],
    [/^bsd[- ]?2([- ]?clause)?$/i, 'BSD-2-Clause'],
    [/^isc$/i, 'ISC'],
    [/^gpl[- ]?3(\.0)?(-only|-or-later)?$/i, 'GPL-3.0'],
    [/^gpl[- ]?2(\.0)?(-only|-or-later)?$/i, 'GPL-2.0'],
    [/^agpl[- ]?3(\.0)?(-only|-or-later)?$/i, 'AGPL-3.0'],
    [/^mpl[- ]?2(\.0)?$/i, 'MPL-2.0'],
    [/^unlicense$/i, 'Unlicense'],
  ];
  for (const [re, spdx] of map) if (re.test(v)) return spdx;
  return v.length > 80 ? v.slice(0, 80) : v;
}

export function detectLicenseText(text) {
  const t = text.slice(0, 4000);
  if (/proprietary|all rights reserved/i.test(t) && /anthropic/i.test(t)) return 'Proprietary';
  if (/Figma Developer Terms/i.test(t)) return FIGMA_TERMS;
  if (/Permission is hereby granted, free of charge/i.test(t) || /^\s*MIT License/im.test(t)) return 'MIT';
  if (/Apache License[\s\S]{0,60}Version 2\.0/i.test(t)) return 'Apache-2.0';
  if (/GNU AFFERO GENERAL PUBLIC LICENSE/i.test(t)) return 'AGPL-3.0';
  if (/GNU LESSER GENERAL PUBLIC LICENSE/i.test(t)) return 'LGPL-3.0';
  if (/GNU GENERAL PUBLIC LICENSE[\s\S]{0,120}Version 3/i.test(t)) return 'GPL-3.0';
  if (/GNU GENERAL PUBLIC LICENSE[\s\S]{0,120}Version 2/i.test(t)) return 'GPL-2.0';
  if (/Mozilla Public License[\s\S]{0,40}2\.0/i.test(t)) return 'MPL-2.0';
  if (/Attribution-ShareAlike 4\.0/i.test(t)) return 'CC-BY-SA-4.0';
  if (/Attribution 4\.0 International|CC-BY-4\.0|CC BY 4\.0/i.test(t)) return 'CC-BY-4.0';
  if (/CC0 1\.0|Creative Commons Zero/i.test(t)) return 'CC0-1.0';
  if (/This is free and unencumbered software/i.test(t)) return 'Unlicense';
  if (/SIL OPEN FONT LICENSE/i.test(t)) return 'OFL-1.1';
  if (/^\s*ISC License/im.test(t)) return 'ISC';
  if (/Redistribution and use in source and binary forms/i.test(t))
    return /Neither the name/i.test(t) ? 'BSD-3-Clause' : 'BSD-2-Clause';
  if (/proprietary|all rights reserved/i.test(t)) return 'Proprietary';
  return null;
}

/** Procura LICENSE* da pasta do item subindo até `stop` (exclusive). */
function licenseFromFiles(dir, stop) {
  let d = dir;
  while (d.startsWith(stop) && d !== stop) {
    for (const f of listFiles(d)) {
      if (/^(LICENSE|LICENCE|COPYING)(\.(txt|md))?$/i.test(f)) {
        const lic = detectLicenseText(readText(path.join(d, f)));
        if (lic) return lic;
      }
    }
    d = path.dirname(d);
  }
  return null;
}

function toList(v) {
  if (v === null || v === undefined) return [];
  if (Array.isArray(v)) return v.map((x) => String(x).trim()).filter(Boolean);
  return String(v)
    .split(/[,\s]+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function authorOf(fm) {
  const a = fm?.author ?? fm?.metadata?.author ?? null;
  if (!a) return null;
  if (typeof a === 'string') return a;
  if (typeof a === 'object' && a.name) return String(a.name);
  return null;
}

/** Remove strings enormes do JSON guardado como frontmatter de mcp/hook/setting. */
function slimJson(v, depth = 0) {
  if (typeof v === 'string') return v.length > 2000 ? v.slice(0, 2000) + '…' : v;
  if (Array.isArray(v)) return v.map((x) => slimJson(x, depth + 1));
  if (v && typeof v === 'object') {
    const o = {};
    for (const [k, x] of Object.entries(v)) o[k] = slimJson(x, depth + 1);
    return o;
  }
  return v;
}

// ---------------------------------------------------------------- construção
class Builder {
  constructor(src, commit, date, sid = 'cct', repo = CCT_REPO) {
    this.src = src;
    this.sid = sid;
    this.repo = repo;
    this.url = `https://github.com/${repo}`;
    this.comp = path.join(src, 'cli-tool', 'components');
    this.commit = commit;
    this.updated = date.slice(0, 10);
    this.items = [];
    this.excluded = [];
    this.skipped = [];
  }

  source(relPath, dirRel) {
    return {
      id: this.sid,
      repo: this.repo,
      commit: this.commit,
      path: relPath,
      dir: dirRel,
      url: `${this.url}/blob/${this.commit}/${relPath}`,
    };
  }

  push(item) {
    item.files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
    item.normalization = [...(item._norm || [])].sort();
    delete item._norm;
    this.items.push(item);
  }

  mk({ kind, typePlural, category, idRest, name, entryAbs, dirAbs, files, fm, description, license, author, tags, origin, references }) {
    const repoRel = (p) => posix(path.relative(this.src, p));
    return {
      id: `${this.sid}:${typePlural}/${idRest}`,
      kind,
      name,
      category,
      description,
      source: this.source(repoRel(entryAbs === dirAbs ? dirAbs : entryAbs), repoRel(dirAbs)),
      license: license ?? 'MIT',
      author: author ?? null,
      tags: tags ?? [],
      origin_tool: origin ?? 'claude',
      files,
      entry: posix(path.relative(dirAbs, entryAbs)) || path.basename(entryAbs),
      frontmatter: fm ?? {},
      references: references ?? [],
      stars: null,
      updated: this.updated,
      security: null,
    };
  }

  /** agentes e comandos: um .md por item */
  markdownKind(typePlural, kind) {
    const base = path.join(this.comp, typePlural);
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      const files = walkFiles(catDir).filter((f) => {
        const rel = posix(path.relative(catDir, f));
        const bn = path.basename(f);
        if (/^readme\.md$/i.test(bn)) {
          this.skipped.push({ path: posix(path.relative(this.src, f)), reason: 'readme' });
          return false;
        }
        if (kind === 'agent') {
          // agentes: .md na pasta da categoria, ou texto sem extensão; scripts auxiliares ficam de fora
          if (rel.includes('/')) {
            this.skipped.push({ path: posix(path.relative(this.src, f)), reason: 'helper_script' });
            return false;
          }
          return bn.endsWith('.md') || !bn.includes('.');
        }
        return bn.endsWith('.md');
      });
      for (const f of files) {
        const norm = new Set();
        const dir = path.dirname(f);
        const bn = path.basename(f);
        const name = bn.replace(/\.md$/, '');
        const text = readText(f);
        const sp = splitFrontmatter(text);
        let fm = {};
        let body = text;
        if (sp) {
          fm = parseYaml(sp.yaml);
          body = sp.body;
        } else {
          norm.add('added_frontmatter');
        }
        if (!bn.endsWith('.md')) norm.add('missing_extension');
        if (!fm.name && kind === 'agent') { fm.name = name; norm.add('added_frontmatter'); }
        let desc = fm.description;
        if (!desc || typeof desc !== 'string' || !desc.trim()) {
          desc = descriptionFromBody(body) || humanize(name);
          fm.description = desc;
          norm.add('added_frontmatter');
        }
        let origin = 'claude';
        if (kind === 'agent') {
          const o = toolsOrigin(fm.tools, fm.model);
          if (o === 'copilot') { origin = 'copilot'; norm.add('copilot_tools'); }
          else if (o === 'mixed') norm.add('foreign_tool_ids');
        }
        if (fm.model !== undefined && origin === 'claude' && !validClaudeModel(fm.model)) {
          fm.model_removed = fm.model;
          delete fm.model;
          norm.add('removed_invalid_model');
        }
        const shortD = shortDescription(desc, norm);
        if (typeof fm.description === 'string') fm.description = cleanDescription(fm.description);
        const relFromCat = posix(path.relative(catDir, f)).replace(/\.md$/, '');
        const nested = relFromCat.includes('/');
        if (nested) norm.add('nested_path_indexed');
        const item = this.mk({
          kind,
          typePlural,
          category,
          idRest: `${category}/${relFromCat}`,
          name,
          entryAbs: f,
          dirAbs: dir,
          files: [fileEntry(dir, f)],
          fm,
          description: shortD,
          license: normalizeLicense(fm.license) || 'MIT',
          author: authorOf(fm),
          tags: [...new Set([...toList(fm.tags), ...(nested ? [relFromCat.split('/')[0]] : [])])],
          origin,
        });
        item._norm = norm;
        this.push(item);
      }
    }
  }

  hooks() {
    const base = path.join(this.comp, 'hooks');
    for (const f of listFiles(base)) {
      this.skipped.push({ path: posix(path.relative(this.src, path.join(base, f))), reason: 'hook_pattern_library' });
    }
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      for (const bn of listFiles(catDir).filter((f) => f.endsWith('.json'))) {
        const f = path.join(catDir, bn);
        const name = bn.replace(/\.json$/, '');
        const norm = new Set();
        const j = readJson(f);
        const set = new Map();
        set.set(f, fileEntry(catDir, f));
        for (const ext of ['.py', '.sh', '.js']) {
          const sib = path.join(catDir, name + ext);
          if (fs.existsSync(sib)) set.set(sib, fileEntry(catDir, sib));
        }
        for (const sf of Array.isArray(j.supportingFiles) ? j.supportingFiles : []) {
          if (!sf?.source) continue;
          const p = path.resolve(catDir, sf.source);
          if (p.startsWith(catDir) && fs.existsSync(p) && fs.statSync(p).isFile()) set.set(p, fileEntry(catDir, p));
          else norm.add('missing_supporting_file');
        }
        // comandos que chamam .claude/hooks/<x>.(py|sh) de outra pasta
        const cmdText = JSON.stringify(j.hooks ?? {});
        for (const m of cmdText.matchAll(/\.claude\/hooks\/([\w.-]+\.(?:py|sh|js))/g)) {
          const p = path.join(catDir, m[1]);
          if (fs.existsSync(p)) set.set(p, fileEntry(catDir, p));
        }
        let desc = j.description;
        if (!desc) { desc = humanize(name); norm.add('added_description'); }
        const item = this.mk({
          kind: 'hook',
          typePlural: 'hooks',
          category,
          idRest: `${category}/${name}`,
          name,
          entryAbs: f,
          dirAbs: catDir,
          files: [...set.values()],
          fm: slimJson(j),
          description: shortDescription(desc, norm),
          license: normalizeLicense(j.license) || 'MIT',
          author: authorOf(j),
          tags: toList(j.tags ?? j.keywords),
        });
        item._norm = norm;
        this.push(item);
      }
    }
  }

  mcps() {
    const base = path.join(this.comp, 'mcps');
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      for (const bn of listFiles(catDir).filter((f) => f.endsWith('.json'))) {
        const f = path.join(catDir, bn);
        const name = bn.replace(/\.json$/, '');
        const norm = new Set();
        const j = readJson(f);
        const servers = j.mcpServers ?? j.servers ?? {};
        let desc = j.description ?? null;
        for (const s of Object.values(servers)) {
          if (!desc && s && typeof s === 'object') {
            if (s.description) desc = s.description;
            else if (s.descrption) { desc = s.descrption; norm.add('fixed_description_typo'); }
          }
        }
        if (!desc) { desc = humanize(name); norm.add('added_description'); }
        const item = this.mk({
          kind: 'mcp',
          typePlural: 'mcps',
          category,
          idRest: `${category}/${name}`,
          name,
          entryAbs: f,
          dirAbs: catDir,
          files: [fileEntry(catDir, f)],
          fm: slimJson(j),
          description: shortDescription(desc, norm),
          license: normalizeLicense(j.license) || 'MIT',
          author: authorOf(j),
          tags: toList(j.tags ?? j.keywords),
        });
        item._norm = norm;
        this.push(item);
      }
    }
  }

  settings() {
    const base = path.join(this.comp, 'settings');
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      for (const bn of listFiles(catDir).filter((f) => f.endsWith('.json'))) {
        const f = path.join(catDir, bn);
        const name = bn.replace(/\.json$/, '');
        const norm = new Set();
        const j = readJson(f);
        const kind = category === 'statusline' ? 'statusline' : 'setting';
        const files = [fileEntry(catDir, f)];
        const cmd = j?.statusLine?.command ?? '';
        for (const m of String(cmd).matchAll(/\.claude\/scripts\/([\w.-]+\.(?:py|sh|js))/g)) {
          const p = path.join(catDir, m[1]);
          if (fs.existsSync(p)) files.push(fileEntry(catDir, p));
          else norm.add('missing_supporting_file');
        }
        if (j.files && typeof j.files === 'object') norm.add('inline_files');
        let desc = j.description;
        if (!desc) { desc = humanize(name); norm.add('added_description'); }
        const item = this.mk({
          kind,
          typePlural: kind === 'statusline' ? 'statuslines' : 'settings',
          category,
          idRest: `${category}/${name}`,
          name,
          entryAbs: f,
          dirAbs: catDir,
          files,
          fm: slimJson(j),
          description: shortDescription(desc, norm),
          license: normalizeLicense(j.license) || 'MIT',
          author: authorOf(j),
          tags: toList(j.tags ?? j.keywords),
        });
        item._norm = norm;
        this.push(item);
      }
    }
  }

  /** Motivo de exclusão de uma skill proprietária (ou null). */
  static skillExclusion(name, license) {
    if (['docx', 'pdf', 'pptx', 'xlsx', 'pdf-anthropic'].includes(name)) return 'anthropic_proprietary_document_skill';
    if (name.endsWith('-official')) return 'anthropic_official_duplicate';
    return nonRedistributable(license);
  }

  /**
   * Skills: toda pasta com SKILL.md abaixo de `base`.
   * opts: `category` fixa (senão a 1ª pasta abaixo de `base`, ou `general`),
   * `defaultLicense` (licença do repo; null = sem licença → fica de fora),
   * `licenseStop` (até onde subir procurando LICENSE*), `excludeNames`
   * ({nome: motivo}; sem ele valem as regras do cct), `origin`, `author`,
   * `codexMetadata` (lê `agents/openai.yaml`).
   */
  skills(base = path.join(this.comp, 'skills'), opts = {}) {
    if (!fs.existsSync(base)) return;
    const o = { defaultLicense: 'MIT', licenseStop: base, ...opts };
    const skillDirs = walkFiles(base)
      .filter((f) => path.basename(f) === 'SKILL.md')
      .map((f) => path.dirname(f))
      .filter((d) => d !== base)
      .sort();
    const isSkillDir = new Set(skillDirs);
    const idRestOf = (rel) => {
      const parts = rel.split('/');
      if (o.category) return `${o.category}/${rel}`;
      return parts.length >= 2 ? rel : `general/${rel}`;
    };
    for (const dir of skillDirs) {
      const rel = posix(path.relative(base, dir));
      const parts = rel.split('/');
      const category = o.category ?? (parts.length >= 2 ? parts[0] : 'general');
      const name = parts[parts.length - 1];
      const norm = new Set();
      const entry = path.join(dir, 'SKILL.md');
      const text = readText(entry);
      const sp = splitFrontmatter(text);
      let fm = {};
      let body = text;
      if (sp) { fm = parseYaml(sp.yaml); body = sp.body; } else norm.add('added_frontmatter');
      if (!fm.name) { fm.name = name; norm.add('added_frontmatter'); }
      if (!fm.description) { fm.description = descriptionFromBody(body) || humanize(name); norm.add('added_frontmatter'); }
      let license = normalizeLicense(fm.license ?? fm.metadata?.license);
      const rawLic = typeof fm.license === 'string' ? fm.license : '';
      if (!license || /complete terms/i.test(rawLic)) {
        const fromFile = licenseFromFiles(dir, o.licenseStop);
        if (fromFile) { license = fromFile; norm.add('license_from_file'); }
      }
      if (/proprietary/i.test(rawLic)) license = 'Proprietary';
      const why = o.excludeNames
        ? o.excludeNames[name] ?? nonRedistributable(license)
        : Builder.skillExclusion(name, license);
      if (why) {
        this.excluded.push({ source: this.sid, path: posix(path.relative(this.src, dir)), name, reason: why, ...(license ? { license } : {}) });
        continue;
      }
      if (!license && o.defaultLicense) { license = o.defaultLicense; norm.add('license_from_repo'); }
      if (!license) {
        this.excluded.push({ source: this.sid, path: posix(path.relative(this.src, dir)), name, reason: 'no_license' });
        continue;
      }
      const nestedSkills = skillDirs.filter((d) => d !== dir && d.startsWith(dir + path.sep));
      const files = walkFiles(dir, { skipDirs: (d) => isSkillDir.has(d) }).map((f) => fileEntry(dir, f));
      if (parts.length > 2) norm.add('nested_skill_indexed');
      if (fm.name && fm.name !== name) norm.add('name_differs_from_dir');
      const tags = [...new Set([...toList(fm.tags), ...toList(fm.metadata?.tags)])];
      const codexYaml = path.join(dir, 'agents', 'openai.yaml');
      if (o.codexMetadata && fs.existsSync(codexYaml)) {
        // metadado de UI do Codex (display_name, ícones, default_prompt, dependências MCP)
        const meta = parseYaml(readText(codexYaml));
        if (meta && typeof meta === 'object' && Object.keys(meta).length) fm['x-codex'] = slimJson(meta);
      }
      const item = this.mk({
        kind: 'skill',
        typePlural: 'skills',
        category,
        idRest: idRestOf(rel),
        name,
        entryAbs: entry,
        dirAbs: dir,
        files,
        fm,
        description: shortDescription(fm.description, norm),
        license,
        author: authorOf(fm) ?? o.author ?? null,
        tags,
        origin: o.origin,
        references: nestedSkills.map((d) => `${this.sid}:skills/${idRestOf(posix(path.relative(base, d)))}`),
      });
      // a pasta da skill é o item: source.path aponta para a pasta
      item.source.path = item.source.dir;
      item.source.url = `${this.url}/tree/${this.commit}/${item.source.dir}`;
      if (fs.existsSync(codexYaml)) item.tags.push('codex-metadata');
      item._norm = norm;
      this.push(item);
    }
  }

  loops() {
    const base = path.join(this.comp, 'loops');
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      for (const bn of listFiles(catDir).filter((f) => f.endsWith('.md'))) {
        const f = path.join(catDir, bn);
        const name = bn.replace(/\.md$/, '');
        const norm = new Set();
        const text = readText(f);
        const sp = splitFrontmatter(text);
        const fm = sp ? parseYaml(sp.yaml) : {};
        if (!sp) norm.add('added_frontmatter');
        if (!fm.description) { fm.description = descriptionFromBody(sp ? sp.body : text) || humanize(name); norm.add('added_frontmatter'); }
        const item = this.mk({
          kind: 'loop',
          typePlural: 'loops',
          category,
          idRest: `${category}/${name}`,
          name,
          entryAbs: f,
          dirAbs: catDir,
          files: [fileEntry(catDir, f)],
          fm,
          description: shortDescription(fm.description, norm),
          license: normalizeLicense(fm.license) || 'MIT',
          author: authorOf(fm),
          tags: toList(fm.tags),
        });
        item._rawRefs = toList(fm.components);
        item._norm = norm;
        this.push(item);
      }
    }
  }

  mods() {
    const base = path.join(this.comp, 'mods');
    for (const category of listDirs(base)) {
      const catDir = path.join(base, category);
      for (const name of listDirs(catDir)) {
        const dir = path.join(catDir, name);
        const manifest = path.join(dir, '.claude-plugin', 'plugin.json');
        if (!fs.existsSync(manifest)) {
          this.skipped.push({ path: posix(path.relative(this.src, dir)), reason: 'mod_without_manifest' });
          continue;
        }
        const norm = new Set();
        const j = readJson(manifest);
        const hooksJson = path.join(dir, 'hooks', 'hooks.json');
        if (!fs.existsSync(hooksJson)) norm.add('missing_hooks_json');
        let license = normalizeLicense(j.license);
        if (!license) {
          license = licenseFromFiles(dir, catDir);
          if (license) norm.add('license_from_file');
        }
        if (!license) { license = 'MIT'; norm.add('license_from_repo'); }
        const files = walkFiles(dir, { skipDot: true }).map((f) => fileEntry(dir, f));
        const item = this.mk({
          kind: 'mod',
          typePlural: 'mods',
          category,
          idRest: `${category}/${name}`,
          name,
          entryAbs: manifest,
          dirAbs: dir,
          files,
          fm: slimJson(j),
          description: shortDescription(j.description || humanize(name), norm),
          license,
          author: authorOf(j),
          tags: toList(j.keywords),
        });
        item.source.path = item.source.dir;
        item.source.url = `${this.url}/tree/${this.commit}/${item.source.dir}`;
        item._norm = norm;
        this.push(item);
      }
      for (const f of listFiles(catDir)) {
        this.skipped.push({ path: posix(path.relative(this.src, path.join(catDir, f))), reason: 'mods_support_file' });
      }
    }
  }

  sandbox() {
    const base = path.join(this.comp, 'sandbox');
    for (const provider of listDirs(base)) {
      const dir = path.join(base, provider);
      const norm = new Set();
      const entryName = ['claude-code-sandbox.md', 'README.md'].find((f) => fs.existsSync(path.join(dir, f)));
      if (!entryName) continue;
      const entry = path.join(dir, entryName);
      const text = readText(entry);
      const sp = splitFrontmatter(text);
      const fm = sp ? parseYaml(sp.yaml) : {};
      if (!fm.description) { fm.description = descriptionFromBody(sp ? sp.body : text) || `${humanize(provider)} sandbox`; norm.add('added_frontmatter'); }
      const files = walkFiles(dir).map((f) => fileEntry(dir, f));
      const item = this.mk({
        kind: 'sandbox',
        typePlural: 'sandbox',
        category: 'sandbox',
        idRest: provider,
        name: provider,
        entryAbs: entry,
        dirAbs: dir,
        files,
        fm,
        description: shortDescription(fm.description, norm),
        license: 'MIT',
        author: authorOf(fm),
        tags: [provider],
      });
      item.source.path = item.source.dir;
      item.source.url = `${this.url}/tree/${this.commit}/${item.source.dir}`;
      item._norm = norm;
      this.push(item);
    }
  }

  templates() {
    const base = path.join(this.src, 'cli-tool', 'templates');
    const one = (dir, category, name, idRest, skipDirs) => {
      const norm = new Set();
      const entryName = ['CLAUDE.md', 'README.md'].find((f) => fs.existsSync(path.join(dir, f)));
      const files = walkFiles(dir, { skipDirs }).map((f) => fileEntry(dir, f));
      if (!files.length) return;
      const entry = entryName ? path.join(dir, entryName) : path.join(dir, files[0].path);
      const text = readText(entry);
      const sp = splitFrontmatter(text);
      const efm = sp ? parseYaml(sp.yaml) : {};
      const d = (typeof efm.description === 'string' && efm.description.trim()) || descriptionFromBody(sp ? sp.body : text) || `${humanize(name)} project template`;
      norm.add('added_frontmatter');
      const item = this.mk({
        kind: 'template',
        typePlural: 'templates',
        category,
        idRest,
        name,
        entryAbs: entry,
        dirAbs: dir,
        files,
        fm: { name, description: d },
        description: shortDescription(d, norm),
        license: 'MIT',
        tags: [category],
      });
      item.source.path = item.source.dir;
      item.source.url = `${this.url}/tree/${this.commit}/${item.source.dir}`;
      item._norm = norm;
      this.push(item);
    };
    for (const lang of listDirs(base)) {
      const dir = path.join(base, lang);
      one(dir, lang, lang, lang, (d) => d === path.join(dir, 'examples'));
      for (const fw of listDirs(path.join(dir, 'examples'))) {
        one(path.join(dir, 'examples', fw), lang, fw, `${lang}/${fw}`, () => false);
      }
    }
  }

  finish() {
    // referências dos loops: `agent:cat/name` → id do índice
    const ids = new Set(this.items.map((i) => i.id));
    const plural = { agent: 'agents', command: 'commands', skill: 'skills', hook: 'hooks', setting: 'settings', mcp: 'mcps', statusline: 'statuslines', mod: 'mods', loop: 'loops' };
    for (const it of this.items) {
      if (!it._rawRefs) continue;
      const refs = [];
      for (const r of it._rawRefs) {
        const m = /^([a-z]+):(.+)$/.exec(r);
        if (!m) continue;
        let cand = `${this.sid}:${plural[m[1]] ?? m[1] + 's'}/${m[2]}`;
        if (!ids.has(cand) && m[1] === 'setting') cand = `${this.sid}:statuslines/${m[2]}`;
        if (ids.has(cand)) refs.push(cand);
        else {
          refs.push(cand);
          it.normalization = [...new Set([...it.normalization, 'unresolved_reference'])].sort();
        }
      }
      it.references = refs;
      delete it._rawRefs;
    }
    // colisões de nome dentro do mesmo tipo (a instalação achata a categoria)
    const byKey = new Map();
    for (const it of this.items) {
      const k = `${it.kind}\u0000${it.name.toLowerCase()}`;
      if (!byKey.has(k)) byKey.set(k, []);
      byKey.get(k).push(it);
    }
    let collisions = 0;
    for (const group of byKey.values()) {
      if (group.length < 2) continue;
      for (const it of group) {
        it.collides_with = group.filter((g) => g !== it).map((g) => g.id);
        it.install_name = `${it.category}-${it.name}`;
        it.normalization = [...new Set([...it.normalization, 'renamed_collision'])].sort();
        collisions++;
      }
    }
    return collisions;
  }
}

// ---------------------------------------------------------------- marketplaces
const CURATED_MARKETPLACES = [
  ['anthropics/claude-plugins-official', null],
  ['anthropics/knowledge-work-plugins', null],
  ['affaan-m/everything-claude-code', null],
  ['thedotmack/claude-mem', null],
  ['jarrodwatts/claude-hud', null],
  ['EveryInc/compound-engineering-plugin', null],
  ['alirezarezvani/claude-skills', null],
  ['davepoon/buildwithclaude', 'https://buildwithclaude.com/'],
  ['Nyanbalaji28/best-claude-skills', 'https://augmentclaude.com/'],
  ['lackeyjb/playwright-skill', null],
  ['nyldn/claude-octopus', null],
  ['jeremylongshore/claude-code-plugins-plus-skills', null],
  ['timescale/pg-aiguide', null],
  ['CloudAI-X/claude-workflow-v2', null],
  ['numman-ali/n-skills', null],
  ['obra/superpowers-marketplace', null],
  ['muratcankoylan/ralph-wiggum-marketer', null],
  ['team-attention/plugins-for-claude-natives', null],
  ['ananddtyagi/cc-marketplace', null],
  ['agent-sh/agentsys', null],
  ['ccplugins/awesome-claude-code-plugins', null],
  ['hamelsmu/claude-review-loop', null],
  ['sangrokjung/claude-forge', null],
  ['gmickel/gmickel-claude-marketplace', null],
  ['kingbootoshi/cartographer', null],
  ['zscole/adversarial-spec', null],
  ['hashicorp/agent-skills', null],
  ['quant-sentiment-ai/claude-equity-research', null],
  ['777genius/claude-notifications-go', null],
  ['Piebald-AI/claude-code-lsps', null],
  ['chu2bard/pinion-os', null],
  ['Airtable/skills', null],
  ['krasserm/ml-plugins', null],
  ['cohesivity-org/cohesivity-plugin', 'https://cohesivity.ai'],
  ['iOSDevSK/html2wp-cc-plugin', 'https://html2wp.dev/'],
  ['curviate/curviate-plugin', 'https://curviate.com/'],
];

let ghAvailable = null;
function hasGh() {
  if (ghAvailable === null) {
    try {
      execFileSync('gh', ['auth', 'status'], { stdio: 'ignore' });
      ghAvailable = true;
    } catch {
      ghAvailable = false;
    }
  }
  return ghAvailable;
}

async function githubApi(p) {
  if (hasGh()) {
    try {
      const out = execFileSync('gh', ['api', p], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 64 << 20 });
      return JSON.parse(out);
    } catch (e) {
      const msg = String(e.stderr || e.message);
      if (/404|Not Found/i.test(msg)) return null;
      console.error(`[catalog] gh api ${p}: ${msg.trim().split('\n')[0]}`);
      return null;
    }
  }
  const r = await fetch(`https://api.github.com/${p}`, { headers: { 'User-Agent': 'omniget-catalog-build', Accept: 'application/vnd.github+json' } });
  if (r.status === 403 || r.status === 429) {
    console.error(`[catalog] rate limit da API do GitHub em ${p}`);
    return null;
  }
  if (!r.ok) return null;
  return r.json();
}

async function rawFile(repo, commit, p) {
  const r = await fetch(`https://raw.githubusercontent.com/${repo}/${commit}/${p}`, { headers: { 'User-Agent': 'omniget-catalog-build' } });
  if (!r.ok) return null;
  return r.text();
}

function lsRemoteHead(repo) {
  try {
    const out = execFileSync('git', ['ls-remote', `https://github.com/${repo}.git`, 'HEAD'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], timeout: 30000 });
    return out.split(/\s/)[0] || null;
  } catch {
    return null;
  }
}

function parseJsonLoose(t) {
  if (t === null || t === undefined) return null;
  try {
    return JSON.parse(t);
  } catch {
    try {
      return JSON.parse(t.replace(/\/\/[^\n]*/g, '').replace(/,(\s*[}\]])/g, '$1'));
    } catch {
      return null;
    }
  }
}

/** Resolve a origem de um plugin de marketplace para {repo, commit, ref, path, url}. */
export function pluginSource(entrySource, mRepo, mCommit, pluginRoot) {
  const clean = (p) => String(p || '').replace(/^\.\/+/, '').replace(/\/+$/, '');
  if (typeof entrySource === 'string') {
    if (/^https?:\/\//.test(entrySource)) return githubUrlSource(entrySource);
    let p = clean(entrySource);
    if (pluginRoot && !String(entrySource).startsWith('./') && !p.includes('/')) p = [clean(pluginRoot), p].filter(Boolean).join('/');
    return { repo: mRepo, commit: mCommit, ref: null, path: p };
  }
  if (entrySource && typeof entrySource === 'object') {
    const kind = entrySource.source;
    if (kind === 'github' && entrySource.repo) {
      return { repo: entrySource.repo, commit: entrySource.sha ?? null, ref: entrySource.ref ?? null, path: clean(entrySource.path) };
    }
    if ((kind === 'url' || kind === 'git') && entrySource.url) {
      const s = githubUrlSource(entrySource.url);
      if (entrySource.sha) s.commit = entrySource.sha;
      if (entrySource.ref) s.ref = entrySource.ref;
      if (entrySource.path) s.path = clean(entrySource.path);
      return s;
    }
    if (kind === 'git-subdir' && entrySource.url) {
      const s = githubUrlSource(entrySource.url);
      s.path = clean(entrySource.path ?? entrySource.subdir);
      if (entrySource.sha) s.commit = entrySource.sha;
      if (entrySource.ref) s.ref = entrySource.ref;
      return s;
    }
    if (kind === 'npm' || entrySource.package) {
      return { repo: null, commit: null, ref: entrySource.version ?? null, path: '', url: `npm:${entrySource.package ?? entrySource.name ?? ''}` };
    }
  }
  return { repo: mRepo, commit: mCommit, ref: null, path: '' };
}

function githubUrlSource(url) {
  const m = /github\.com[/:]([^/]+)\/([^/#?]+?)(?:\.git)?(?:\/tree\/([^/]+)\/?(.*))?$/.exec(url);
  if (m) return { repo: `${m[1]}/${m[2]}`, commit: null, ref: m[3] ?? null, path: m[4] ?? '' };
  return { repo: null, commit: null, ref: null, path: '', url };
}

async function marketplaces(network, collected) {
  const out = [];
  if (!network) return out;
  for (const [repo, website] of CURATED_MARKETPLACES) {
    const info = await githubApi(`repos/${repo}`);
    const branch = info?.default_branch ?? 'HEAD';
    let commit = null;
    const c = await githubApi(`repos/${repo}/commits/${branch}`);
    commit = c?.sha ?? lsRemoteHead(repo);
    if (!commit) {
      console.error(`[catalog] ${repo}: sem commit (repo sumiu?)`);
      out.push({ repo, name: repo.split('/')[1], kind: 'unavailable', description: null, stars: null, license: null, updated: null, commit: null, website, plugins: 0, url: `https://github.com/${repo}` });
      continue;
    }
    const marketText = await rawFile(repo, commit, '.claude-plugin/marketplace.json');
    const market = parseJsonLoose(marketText);
    const pluginText = market ? null : await rawFile(repo, commit, '.claude-plugin/plugin.json');
    const single = parseJsonLoose(pluginText);
    const repoLicense = normalizeLicense(info?.license?.spdx_id && info.license.spdx_id !== 'NOASSERTION' ? info.license.spdx_id : null);
    const stars = typeof info?.stargazers_count === 'number' ? info.stargazers_count : null;
    const updated = (info?.pushed_at ?? c?.commit?.committer?.date ?? '').slice(0, 10) || null;
    const entries = [];
    if (market && Array.isArray(market.plugins)) {
      for (const p of market.plugins) if (p && p.name) entries.push({ p, src: pluginSource(p.source, repo, commit, market.metadata?.pluginRoot) });
    } else if (single && single.name) {
      entries.push({ p: single, src: { repo, commit, ref: null, path: '' } });
    }
    const mName = market?.name ?? single?.name ?? repo.split('/')[1];
    for (const { p, src } of entries) {
      const norm = new Set();
      if (src.repo && !src.commit) norm.add('unpinned_source');
      const category = typeof p.category === 'string' && p.category.trim() ? p.category.trim().toLowerCase().replace(/\s+/g, '-') : 'general';
      const desc = shortDescription(p.description || market?.metadata?.description || info?.description || humanize(p.name), norm);
      const author = typeof p.author === 'string' ? p.author : p.author?.name ?? market?.owner?.name ?? repo.split('/')[0];
      const dir = src.path || '';
      collected.push({
        id: `${repo}:plugins/${category}/${p.name}`,
        kind: 'plugin',
        name: String(p.name),
        category,
        description: desc,
        source: {
          id: repo,
          repo: src.repo,
          commit: src.commit,
          ref: src.ref,
          path: dir ? `${dir}/.claude-plugin/plugin.json` : '.claude-plugin/plugin.json',
          dir,
          url: src.repo ? `https://github.com/${src.repo}/tree/${src.commit ?? src.ref ?? 'HEAD'}/${dir}`.replace(/\/$/, '') : src.url ?? null,
          marketplace: repo,
          marketplace_name: mName,
          ...(src.url ? { upstream: src.url } : {}),
        },
        license: normalizeLicense(p.license) ?? (src.repo === repo ? repoLicense : null),
        author,
        tags: [...new Set([...toList(p.keywords), ...toList(p.tags)])],
        origin_tool: 'claude',
        files: [],
        entry: '.claude-plugin/plugin.json',
        frontmatter: slimJson(p),
        references: [],
        stars: src.repo === repo ? stars : null,
        updated,
        normalization: [...norm].sort(),
        security: null,
      });
    }
    out.push({
      repo,
      name: mName,
      kind: market ? 'marketplace' : single ? 'plugin' : 'none',
      description: shortDescription(market?.metadata?.description ?? single?.description ?? info?.description ?? '', null) || null,
      stars,
      license: repoLicense,
      updated,
      commit,
      website: website ?? info?.homepage ?? null,
      plugins: entries.length,
      url: `https://github.com/${repo}`,
    });
    console.error(`[catalog] ${repo}: ${entries.length} plugin(s), ${stars ?? '?'}★`);
  }
  return out;
}

// ---------------------------------------------------------------- dedup entre fontes
const WORD_RE = /[a-z0-9][a-z0-9_-]{3,}/g;
function wordSet(text) {
  const sp = splitFrontmatter(text);
  return new Set(((sp ? sp.body : text).toLowerCase().match(WORD_RE)) ?? []);
}
function jaccard(a, b) {
  if (!a.size || !b.size) return 0;
  let inter = 0;
  for (const w of a) if (b.has(w)) inter++;
  return inter / (a.size + b.size - inter);
}
const PROVENANCE = {
  'anthropics/skills': /anthropic/i,
  'openai/skills': /\bopenai\b/i,
  'google/skills': /google\/skills|\bgoogle\b/i,
  'obra/superpowers': /superpowers|\bobra\b/i,
  'K-Dense-AI/scientific-agent-skills': /k-dense|scientific-(agent-)?skills/i,
};
function provenanceText(fm) {
  const v = [fm?.source, fm?.author, fm?.repository, fm?.repo, fm?.metadata?.author, fm?.metadata?.source];
  return v.filter((x) => typeof x === 'string').join(' ');
}
const ALIAS_MIN_SIMILARITY = 0.5;
/**
 * Linhagem documentada no README/atribuição do cct: onde cada repo foi
 * vendorizado. Com ela, o mesmo nome basta (as cópias do cct divergiram do
 * original, então o texto sozinho não casa).
 */
const LINEAGE = {
  'anthropics/skills': () => true,
  'openai/skills': () => true,
  'obra/superpowers': () => true,
  'K-Dense-AI/scientific-agent-skills': (cctItem) => cctItem.category === 'scientific',
};

/**
 * A mesma skill vendorizada no cct e no repo original: fica a do repo original;
 * a do cct continua resolvível pelo id, marcada como alias (`alias_of`, tag
 * `alias`, `collides_with` → original, normalização `alias_of_upstream`).
 */
function markAliases(cctItems, cctRoot, upstream) {
  const byName = new Map();
  for (const { item, root } of upstream) {
    if (item.kind !== 'skill') continue;
    const k = item.name.toLowerCase();
    if (!byName.has(k)) byName.set(k, []);
    byName.get(k).push({ item, root });
  }
  const readSkill = (root, it) => {
    try {
      return readText(path.join(root, it.source.dir, 'SKILL.md'));
    } catch {
      return '';
    }
  };
  const report = [];
  for (const it of cctItems) {
    if (it.kind !== 'skill') continue;
    const cands = byName.get(it.name.toLowerCase());
    if (!cands) continue;
    const mySha = it.files.find((f) => f.path === 'SKILL.md')?.sha256;
    const myWords = wordSet(readSkill(cctRoot, it));
    const prov = provenanceText(it.frontmatter);
    let best = null;
    for (const c of cands) {
      const sha = c.item.files.find((f) => f.path === 'SKILL.md')?.sha256;
      const same = !!mySha && mySha === sha;
      const sim = same ? 1 : jaccard(myWords, wordSet(readSkill(c.root, c.item)));
      const byProv = !!prov && PROVENANCE[c.item.source.id]?.test(prov);
      const byLineage = !!LINEAGE[c.item.source.id]?.(it);
      const score = (same ? 2 : 0) + (byProv ? 1 : 0) + (byLineage ? 0.5 : 0) + sim;
      if ((same || byProv || byLineage || sim >= ALIAS_MIN_SIMILARITY) && (!best || score > best.score)) {
        best = { c, score, same, sim, byProv, byLineage };
      }
    }
    if (!best) {
      report.push({ id: it.id, candidates: cands.map((c) => c.item.id), alias: null });
      continue;
    }
    const up = best.c.item;
    it.alias_of = up.id;
    it.tags = [...new Set([...it.tags, 'alias'])];
    it.collides_with = [...new Set([...(it.collides_with ?? []), up.id])];
    if (!it.install_name) it.install_name = `${it.category}-${it.name}`;
    it.normalization = [...new Set([...it.normalization, 'alias_of_upstream'])].sort();
    up.aliases = [...new Set([...(up.aliases ?? []), it.id])].sort();
    report.push({ id: it.id, alias: up.id, match: best.same ? 'identical' : best.byProv ? 'provenance' : best.sim >= ALIAS_MIN_SIMILARITY ? 'similar' : 'lineage', similarity: Math.round(best.sim * 100) / 100 });
  }
  return report;
}

/** Colisão de nome entre fontes diferentes (a instalação achata fonte e categoria). */
function crossSourceCollisions(items) {
  const groups = new Map();
  for (const it of items) {
    if (it.kind === 'plugin' || it.alias_of) continue;
    const k = `${it.kind}\u0000${it.name.toLowerCase()}`;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(it);
  }
  let n = 0;
  for (const g of groups.values()) {
    if (new Set(g.map((i) => i.source.id)).size < 2) continue;
    for (const it of g) {
      const others = g.filter((x) => x !== it && x.source.id !== it.source.id).map((x) => x.id);
      it.collides_with = [...new Set([...(it.collides_with ?? []), ...others])];
      if (!it.install_name) it.install_name = `${it.category}-${it.name}`;
      it.normalization = [...new Set([...it.normalization, 'renamed_collision'])].sort();
      n++;
    }
  }
  return n;
}

/** Hash do conteúdo sem campos voláteis (data, commit, estrelas): muda só se o índice mudou de verdade. */
function structuralHash(items, markets) {
  const strip = (it) => {
    const { stars, updated, security, ...rest } = it;
    const { commit, url, ...src } = it.source ?? {};
    return { ...rest, source: src };
  };
  const m = markets.map(({ stars, updated, commit, ...rest }) => rest);
  return sha256(JSON.stringify({ items: items.map(strip), marketplaces: m }));
}

const dirName = (repo) => repo.replace('/', '__');

function countBy(items, key) {
  const o = {};
  for (const it of items) {
    const k = key(it);
    o[k] = (o[k] ?? 0) + 1;
  }
  return o;
}

// ---------------------------------------------------------------- main
async function main() {
  const opt = args();
  const pins = readPins();
  fs.mkdirSync(opt.work, { recursive: true });
  const resolve = (repo) => {
    if (opt.latest) {
      const c = lsRemoteHead(repo);
      if (c) return c;
      console.error(`[catalog] ${repo}: ls-remote falhou; usando o pin`);
    }
    return pins[repo] ?? null;
  };
  let prevMeta = null;
  try {
    prevMeta = JSON.parse(fs.readFileSync(path.join(opt.out, 'catalog-meta.json'), 'utf8'));
  } catch {
    /* primeiro build */
  }

  // claude-code-templates
  const cctSrc = opt.src ?? path.join(opt.work, dirName(CCT_REPO));
  const { commit, date } = ensureCheckout(cctSrc, opt.commit ?? resolve(CCT_REPO) ?? DEFAULT_COMMIT);
  const b = new Builder(cctSrc, commit, date);
  b.markdownKind('agents', 'agent');
  b.markdownKind('commands', 'command');
  b.hooks();
  b.mcps();
  b.settings();
  b.skills();
  // skills soltas do plugin raiz (`.claude-plugin/skills/*`, ex.: owasp-security)
  b.skills(path.join(cctSrc, '.claude-plugin', 'skills'));
  b.loops();
  b.mods();
  b.sandbox();
  b.templates();
  b.finish();
  const newPins = { [CCT_REPO]: commit };
  const cctInfo = opt.network ? await githubApi(`repos/${CCT_REPO}`) : null;
  const sources = [
    {
      id: 'cct',
      name: 'claude-code-templates',
      repo: CCT_REPO,
      commit,
      commit_date: date,
      url: CCT_URL,
      license: 'MIT',
      stars: cctInfo?.stargazers_count ?? null,
      attribution: 'Components from davila7/claude-code-templates (MIT), each keeping its own license and author. Anthropic proprietary document skills are excluded; skills vendored from an indexed upstream repo are aliases of the upstream item.',
    },
  ];

  // repos de skills
  const upstream = [];
  const excluded = [...b.excluded];
  const skipped = [...b.skipped];
  if (!opt.onlyCct) {
    for (const s of SKILL_SOURCES) {
      const c = resolve(s.repo);
      if (!c) {
        console.error(`[catalog] ${s.repo}: sem commit pinado; fonte pulada`);
        continue;
      }
      const dir = path.join(opt.work, dirName(s.repo));
      const co = checkout(s.repo, c, dir);
      const sb = new Builder(dir, co.commit, co.date, s.id, s.repo);
      for (const r of s.roots) {
        sb.skills(path.join(dir, r.dir), {
          category: r.category,
          defaultLicense: s.license,
          licenseStop: dir,
          excludeNames: s.excludeNames ?? {},
          origin: s.origin,
          author: s.author,
          codexMetadata: s.codexMetadata,
        });
      }
      sb.finish();
      const info = opt.network ? await githubApi(`repos/${s.repo}`) : null;
      const stars = typeof info?.stargazers_count === 'number' ? info.stargazers_count : null;
      for (const it of sb.items) {
        it.stars = stars;
        upstream.push({ item: it, root: dir });
      }
      excluded.push(...sb.excluded);
      skipped.push(...sb.skipped.map((x) => ({ source: s.id, ...x })));
      newPins[s.repo] = co.commit;
      sources.push({
        id: s.id,
        name: s.name,
        repo: s.repo,
        commit: co.commit,
        commit_date: co.date,
        url: `https://github.com/${s.repo}`,
        license: s.license ?? 'per-item',
        stars,
        items: sb.items.length,
        excluded: sb.excluded.length,
        ...(s.formerly ? { formerly: s.formerly } : {}),
        attribution: `Skills from ${s.repo}, each keeping its own license and author.${s.note ? ' ' + s.note : ''}`,
      });
      console.error(`[catalog] ${s.repo}@${co.commit.slice(0, 7)}: ${sb.items.length} skill(s), ${sb.excluded.length} excluída(s)`);
    }
  }
  const aliasReport = markAliases(b.items, cctSrc, upstream);
  const baseItems = [...b.items, ...upstream.map((u) => u.item)];
  const crossCollisions = crossSourceCollisions(baseItems);

  const pluginItems = [];
  const markets = await marketplaces(opt.network, pluginItems);
  // colisões de id entre marketplaces (mesmo plugin listado duas vezes na mesma categoria)
  const seen = new Map();
  for (const it of pluginItems) {
    let id = it.id;
    let n = 2;
    while (seen.has(id)) id = `${it.id}~${n++}`;
    if (id !== it.id) it.normalization = [...new Set([...it.normalization, 'renamed_collision'])].sort();
    it.id = id;
    seen.set(id, true);
  }

  const items = [...baseItems, ...pluginItems].sort((x, y) => x.id.localeCompare(y.id));
  const ids = new Set();
  for (const it of items) {
    if (ids.has(it.id)) throw new Error(`id duplicado: ${it.id}`);
    ids.add(it.id);
  }
  const counts = countBy(items, (i) => i.kind);
  const bySource = {};
  for (const it of items) {
    const sid = it.source.marketplace ? 'marketplaces' : it.source.id;
    bySource[sid] ??= {};
    bySource[sid][it.kind] = (bySource[sid][it.kind] ?? 0) + 1;
  }
  const normCounts = {};
  for (const it of items) for (const n of it.normalization) normCounts[n] = (normCounts[n] ?? 0) + 1;
  const origin = countBy(items, (i) => i.origin_tool);
  const licenses = countBy(items, (i) => i.license ?? 'unknown');

  const generated = new Date().toISOString();
  const index = { format: 'omniget-agentkit-catalog', version: 1, generated, sources, items, marketplaces: markets };
  const json = JSON.stringify(index);
  const gzBytes = zlib.gzipSync(json, { level: 9 });
  fs.mkdirSync(opt.out, { recursive: true });
  const gzPath = path.join(opt.out, 'catalog-index.json.gz');
  fs.writeFileSync(gzPath, gzBytes);
  // o JSON puro não vai para static/ (inflava o bundle): só para inspeção
  const plain = path.join(opt.out, 'catalog-index.json');
  if (fs.existsSync(plain)) fs.unlinkSync(plain);
  fs.mkdirSync(opt.inspect, { recursive: true });
  fs.writeFileSync(path.join(opt.inspect, 'catalog-index.json'), json);
  fs.writeFileSync(path.join(opt.inspect, 'aliases.json'), JSON.stringify(aliasReport, null, 2) + '\n');

  const contentSha = structuralHash(items, markets);
  const aliasCount = items.filter((i) => i.alias_of).length;
  const meta = {
    format: 'omniget-agentkit-catalog-meta',
    version: 1,
    generated,
    source: sources[0],
    sources,
    skipped_sources: SKIPPED_SOURCES,
    counts,
    by_source: bySource,
    total: items.length,
    origin_tool: origin,
    licenses,
    normalization: normCounts,
    collisions: items.filter((i) => i.collides_with?.length).length,
    cross_source_collisions: crossCollisions,
    aliases: aliasCount,
    excluded,
    skipped,
    marketplaces: { repos: markets.length, with_plugins: markets.filter((m) => m.plugins > 0).length, plugins: pluginItems.length },
    index: {
      file: 'catalog-index.json.gz',
      bytes: gzBytes.length,
      sha256: sha256(gzBytes),
      json_bytes: Buffer.byteLength(json),
      json_sha256: sha256(json),
      content_sha256: contentSha,
    },
  };
  fs.writeFileSync(path.join(opt.out, 'catalog-meta.json'), JSON.stringify(meta, null, 2) + '\n');
  if (opt.latest || !fs.existsSync(PINS_FILE)) fs.writeFileSync(PINS_FILE, JSON.stringify(newPins, null, 2) + '\n');

  // resumo (corpo do PR do workflow)
  const prevTotal = prevMeta?.total ?? null;
  const kinds = [...new Set(Object.values(bySource).flatMap((o) => Object.keys(o)))].sort();
  const lines = [
    `Catalog index regenerated on ${generated.slice(0, 10)}.`,
    '',
    `- Total: **${items.length}** items${prevTotal !== null ? ` (was ${prevTotal}, ${items.length - prevTotal >= 0 ? '+' : ''}${items.length - prevTotal})` : ''}`,
    `- Index: ${(gzBytes.length / 1024 / 1024).toFixed(2)} MB gz (${(Buffer.byteLength(json) / 1024 / 1024).toFixed(2)} MB JSON)`,
    `- Aliases (cct copies of upstream skills): ${aliasCount}; excluded for license: ${excluded.length}`,
    `- Content hash: \`${contentSha.slice(0, 16)}\`${prevMeta?.index?.content_sha256 ? ` (was \`${prevMeta.index.content_sha256.slice(0, 16)}\`)` : ''}`,
    '',
    '| Source | Commit | ' + kinds.join(' | ') + ' |',
    '|---|---|' + kinds.map(() => '---:').join('|') + '|',
  ];
  for (const [sid, o] of Object.entries(bySource).sort()) {
    const src = sources.find((x) => x.id === sid);
    const c = src ? `[\`${src.commit.slice(0, 7)}\`](https://github.com/${src.repo}/commit/${src.commit})` : `${markets.length} repos`;
    lines.push(`| ${sid} | ${c} | ` + kinds.map((k) => o[k] ?? '').join(' | ') + ' |');
  }
  lines.push('', 'Generated by `.github/workflows/agentkit-catalog.yml` (`node scripts/agentkit-catalog/build.mjs --latest`).');
  fs.writeFileSync(path.join(opt.inspect, 'summary.txt'), lines.join('\n') + '\n');

  console.error(`[catalog] ${items.length} itens → ${gzPath} (${(gzBytes.length / 1024 / 1024).toFixed(2)} MB gz, ${(Buffer.byteLength(json) / 1024 / 1024).toFixed(2)} MB JSON)`);
  console.error(`[catalog] por tipo: ${JSON.stringify(counts)}`);
  console.error(`[catalog] por fonte: ${JSON.stringify(bySource)}`);
  console.error(`[catalog] excluídos: ${excluded.length}; aliases: ${aliasCount}; colisões entre fontes: ${crossCollisions}; content ${contentSha.slice(0, 12)}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
