#!/usr/bin/env node
// Generates the Rust types of the Codex `app-server` JSON-RPC protocol from
// the JSON Schema bundle the CLI itself exports, so the driver follows the
// version the user has instead of a hand-written copy.
//
//   node scripts/codex-schema/gen.mjs                      # npx @openai/codex@latest
//   node scripts/codex-schema/gen.mjs --codex /path/codex  # a local binary
//   node scripts/codex-schema/gen.mjs --schema DIR --version 0.156.0
//   node scripts/codex-schema/gen.mjs --check              # fail if the output would change
//
// The schema bundle comes from `codex app-server generate-json-schema --out DIR`
// (stable surface; `--experimental` is not used on purpose: the driver only
// relies on methods the CLI promises to keep). Output:
// `src-tauri/omniget-core/src/core/llm/drivers/codex/protocol.rs`.
//
// Only node builtins. The generator emits the closure of the types reachable
// from ROOTS (the requests the driver sends, every server request and the
// notifications the translator reads) plus the full method tables. Every
// string enum gets an `Unknown` catch-all and every externally tagged union is
// wrapped so a newer CLI can add a variant without breaking the decode.

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');
const OUT = join(repo, 'src-tauri', 'omniget-core', 'src', 'core', 'llm', 'drivers', 'codex', 'protocol.rs');

// ── CLI args ───────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const check = args.includes('--check');
let schemaDir = opt('--schema');
let version = opt('--version');
const codexBin = opt('--codex');
const out = opt('--out') ?? OUT;
let tmp;

function run(cmd, argv) {
  return execFileSync(cmd, argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], shell: process.platform === 'win32' });
}

if (!schemaDir) {
  tmp = mkdtempSync(join(tmpdir(), 'omniget-codex-schema-'));
  schemaDir = join(tmp, 'schema');
  const [cmd, pre] = codexBin ? [codexBin, []] : ['npx', ['-y', '@openai/codex@latest']];
  run(cmd, [...pre, 'app-server', 'generate-json-schema', '--out', schemaDir]);
  if (!version) {
    const v = run(cmd, [...pre, '--version']).trim();
    version = (v.match(/(\d+\.\d+\.\d+[^\s]*)/) || [])[1] || v;
  }
}
if (!version) version = 'unknown';

// ── Load the bundle ────────────────────────────────────────────────────────

const bundlePath = join(schemaDir, 'codex_app_server_protocol.schemas.json');
if (!existsSync(bundlePath)) {
  console.error(`missing ${bundlePath}: pass --schema <dir from generate-json-schema>`);
  process.exit(2);
}
const bundleText = readFileSync(bundlePath, 'utf8');
const bundle = JSON.parse(bundleText);
const defs = {};
for (const [k, v] of Object.entries(bundle.definitions.v2 || {})) defs[k] = v;
for (const [k, v] of Object.entries(bundle.definitions)) if (k !== 'v2') defs[k] = v;

const table = (file) => {
  const doc = JSON.parse(readFileSync(join(schemaDir, file), 'utf8'));
  return doc.oneOf.map((o) => ({
    method: o.properties.method.enum[0],
    params: o.properties.params ? refName(o.properties.params.$ref) : null,
  }));
};

function refName(ref) {
  if (!ref) return null;
  const parts = ref.split('/');
  return parts[parts.length - 1];
}

const clientRequests = table('ClientRequest.json');
const clientNotifications = table('ClientNotification.json');
const serverRequests = table('ServerRequest.json');
const serverNotifications = table('ServerNotification.json');

const responseOf = (params) => (params ? params.replace(/Params$/, 'Response') : null);

// ── Roots ──────────────────────────────────────────────────────────────────

const CLIENT_METHODS_USED = [
  'initialize',
  'thread/start',
  'thread/resume',
  'thread/fork',
  'thread/read',
  'thread/revert',
  'thread/turns/list',
  'thread/compact/start',
  'turn/start',
  'turn/steer',
  'turn/interrupt',
  'account/read',
  'account/rateLimits/read',
  'model/list',
];
const NOTIFICATIONS_USED = [
  'error',
  'warning',
  'thread/started',
  'thread/status/changed',
  'thread/closed',
  'thread/name/updated',
  'thread/tokenUsage/updated',
  'turn/started',
  'turn/completed',
  'turn/diff/updated',
  'turn/plan/updated',
  'item/started',
  'item/completed',
  'item/agentMessage/delta',
  'item/plan/delta',
  'item/reasoning/summaryTextDelta',
  'item/reasoning/summaryPartAdded',
  'item/reasoning/textDelta',
  'item/commandExecution/outputDelta',
  'item/commandExecution/terminalInteraction',
  'item/fileChange/outputDelta',
  'item/fileChange/patchUpdated',
  'item/mcpToolCall/progress',
  'serverRequest/resolved',
  'account/updated',
  'account/rateLimits/updated',
  'mcpServer/oauthLogin/completed',
  'model/rerouted',
  'deprecationNotice',
  'configWarning',
  'hook/started',
  'hook/completed',
];

const roots = new Set(['InitializeResponse', 'GetAccountRateLimitsResponse', 'JSONRPCErrorError']);
for (const r of clientRequests) {
  if (!CLIENT_METHODS_USED.includes(r.method)) continue;
  if (r.params) roots.add(r.params);
  const resp = responseOf(r.params);
  if (resp && defs[resp]) roots.add(resp);
}
for (const r of serverRequests) {
  if (r.params) roots.add(r.params);
  const resp = responseOf(r.params);
  if (resp && defs[resp]) roots.add(resp);
}
for (const n of serverNotifications) {
  if (NOTIFICATIONS_USED.includes(n.method) && n.params) roots.add(n.params);
}
for (const m of [...CLIENT_METHODS_USED, ...NOTIFICATIONS_USED]) {
  const known = [...clientRequests, ...serverNotifications].some((r) => r.method === m);
  if (!known) console.warn(`warning: method ${m} is not in this schema`);
}

// ── Rust naming ────────────────────────────────────────────────────────────

const KEYWORDS = new Set(
  'as async await break const continue dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return static struct trait true type unsafe use where while abstract become box do final macro override priv typeof unsized virtual yield try gen'.split(' '),
);
const NON_RAW = new Set(['self', 'Self', 'super', 'crate']);

function snake(name) {
  let s = name
    .replace(/[^A-Za-z0-9]+/g, '_')
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase()
    .replace(/^_+|_+$/g, '');
  if (!s) s = 'field';
  if (/^[0-9]/.test(s)) s = `f_${s}`;
  if (NON_RAW.has(s)) return `${s}_`;
  if (KEYWORDS.has(s)) return `r#${s}`;
  return s;
}

function pascal(name) {
  let s = String(name)
    .replace(/[^A-Za-z0-9]+/g, ' ')
    .trim()
    .split(/\s+/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join('');
  if (!s) s = 'Empty';
  if (/^[0-9]/.test(s)) s = `V${s}`;
  if (s === 'Self') s = 'SelfValue';
  return s;
}

function typeName(name) {
  // `Item/commandExecution/...`-style titles never reach here; definitions are
  // already PascalCase. Keep them, only make them legal.
  return pascal(name);
}

// ── Model ──────────────────────────────────────────────────────────────────

/** name -> { kind, doc, ... } */
const items = new Map();
const queue = [];
function want(name) {
  if (!name) return;
  if (!items.has(name) && !queue.includes(name)) queue.push(name);
}

const isNull = (s) => s && (s.type === 'null' || (Array.isArray(s.enum) && s.enum.length === 1 && s.enum[0] === null));

function stripNull(schema) {
  // Returns [inner, nullable].
  if (!schema || schema === true) return [schema, false];
  if (Array.isArray(schema.type) && schema.type.includes('null')) {
    const rest = schema.type.filter((t) => t !== 'null');
    return [{ ...schema, type: rest.length === 1 ? rest[0] : rest }, true];
  }
  for (const key of ['anyOf', 'oneOf']) {
    if (Array.isArray(schema[key]) && schema[key].some(isNull)) {
      const rest = schema[key].filter((s) => !isNull(s));
      if (rest.length === 1) {
        const merged = { ...schema, ...rest[0] };
        delete merged[key];
        if (rest[0][key]) merged[key] = rest[0][key];
        return [merged, true];
      }
      return [{ ...schema, [key]: rest }, true];
    }
  }
  return [schema, false];
}

const DEFAULTABLE = /^(String|bool|i8|i16|i32|i64|u8|u16|u32|u64|f64|serde_json::Value|Vec<.*>|BTreeMap<.*>|Option<.*>)$/;

/**
 * Rust type for a schema in field position. `ctx` names inline types.
 * Returns { ty, named } where `named` is a generated type referenced directly
 * (used for recursion boxing).
 */
function fieldType(schema, ctx) {
  if (schema === true || schema === undefined || schema === null) return { ty: 'serde_json::Value' };
  if (typeof schema !== 'object' || Object.keys(schema).filter((k) => !['description', 'default', 'title'].includes(k)).length === 0) {
    return { ty: 'serde_json::Value' };
  }
  const [inner, nullable] = stripNull(schema);
  if (nullable) {
    const t = fieldType(inner, ctx);
    return { ty: `Option<${t.ty}>`, named: t.named, optional: true };
  }
  if (schema.$ref) {
    const n = refName(schema.$ref);
    want(n);
    return { ty: typeName(n), named: typeName(n) };
  }
  if (Array.isArray(schema.allOf) && schema.allOf.length === 1) return fieldType({ ...schema, ...schema.allOf[0], allOf: undefined }, ctx);
  if (schema.oneOf || schema.anyOf || (Array.isArray(schema.enum) && schema.enum.length > 1) || (schema.properties && Object.keys(schema.properties).length)) {
    const n = ctx;
    if (!items.has(n)) define(n, schema, true);
    return { ty: typeName(n), named: typeName(n) };
  }
  const t = Array.isArray(schema.type) ? schema.type[0] : schema.type;
  switch (t) {
    case 'string':
      return { ty: 'String' };
    case 'boolean':
      return { ty: 'bool' };
    case 'integer':
      return {
        ty: { int64: 'i64', uint64: 'u64', int32: 'i32', uint32: 'u32', uint16: 'u16', uint8: 'u8', int16: 'i16', uint: 'u64', int: 'i64' }[schema.format] || 'i64',
      };
    case 'number':
      return { ty: 'f64' };
    case 'array': {
      const it = fieldType(schema.items, `${ctx}Item`);
      return { ty: `Vec<${it.ty}>` };
    }
    case 'object': {
      if (schema.additionalProperties && schema.additionalProperties !== true) {
        const it = fieldType(schema.additionalProperties, `${ctx}Value`);
        return { ty: `BTreeMap<String, ${it.ty}>` };
      }
      return { ty: 'serde_json::Value' };
    }
    default:
      return { ty: 'serde_json::Value' };
  }
}

function fieldsOf(schema, ctx, skip = []) {
  const req = new Set(schema.required || []);
  const out = [];
  for (const [key, sub] of Object.entries(schema.properties || {})) {
    if (skip.includes(key)) continue;
    const f = fieldType(sub, `${ctx}${pascal(key)}`);
    let ty = f.ty;
    let optional = !!f.optional;
    if (!req.has(key) && !optional) {
      ty = `Option<${ty}>`;
      optional = true;
    }
    out.push({ key, rust: snake(key), ty, optional, named: f.named, doc: sub && sub.description });
  }
  return out;
}

function stringEnumValues(schema) {
  // A plain enum, or a oneOf/anyOf of single-string enums (with descriptions).
  if (Array.isArray(schema.enum) && schema.enum.every((v) => typeof v === 'string')) return schema.enum;
  const branches = schema.oneOf || schema.anyOf;
  if (!branches) return null;
  const vals = [];
  for (const b of branches) {
    if (Array.isArray(b.enum) && b.enum.every((v) => typeof v === 'string')) vals.push(...b.enum);
    else return null;
  }
  return vals;
}

function tagOf(branches) {
  for (const tag of ['type', 'mode', 'kind', 'method']) {
    if (
      branches.every(
        (b) =>
          b &&
          b.properties &&
          b.properties[tag] &&
          Array.isArray(b.properties[tag].enum) &&
          b.properties[tag].enum.length === 1 &&
          !b.oneOf,
      )
    )
      return tag;
  }
  return null;
}

function define(name, schema, inline = false) {
  if (items.has(name)) return;
  const tn = typeName(name);
  const doc = schema && (schema.description || (inline ? null : schema.title !== name ? schema.title : null));
  items.set(name, { kind: 'pending' });
  const [inner, nullable] = stripNull(schema);
  if (nullable) {
    const t = fieldType(inner, `${name}Inner`);
    items.set(name, { kind: 'alias', tn, ty: `Option<${t.ty}>`, doc });
    return;
  }
  if (schema === true || !schema || typeof schema !== 'object') {
    items.set(name, { kind: 'alias', tn, ty: 'serde_json::Value', doc });
    return;
  }
  if (schema.$ref) {
    const t = fieldType(schema, name);
    items.set(name, { kind: 'alias', tn, ty: t.ty, doc });
    return;
  }
  if (Array.isArray(schema.allOf) && schema.allOf.length === 1 && !schema.properties) {
    const t = fieldType(schema.allOf[0], `${name}Inner`);
    items.set(name, { kind: 'alias', tn, ty: t.ty, doc });
    return;
  }
  const strings = stringEnumValues(schema);
  if (strings) {
    items.set(name, { kind: 'strenum', tn, values: [...new Set(strings)], doc });
    return;
  }
  const branches = schema.oneOf || schema.anyOf;
  if (branches && !schema.properties) {
    const tag = tagOf(branches);
    if (tag) {
      const variants = branches.map((b) => {
        const value = b.properties[tag].enum[0];
        const vn = pascal(value);
        const fields = fieldsOf(b, `${name}${vn}`, [tag]);
        // A branch with its own `anyOf` (image input: `url` or `fileId`):
        // every alternative's properties become optional fields.
        for (const alt of b.anyOf || []) {
          for (const f of fieldsOf({ ...alt, required: [] }, `${name}${vn}`, [tag])) {
            if (!fields.some((g) => g.key === f.key)) fields.push(f);
          }
        }
        return { value, vn, fields, doc: b.description };
      });
      items.set(name, { kind: 'tagged', tn, tag, variants, doc });
      return;
    }
    // Externally tagged: bare strings are unit variants, objects with exactly
    // one required key are data variants.
    const ext = [];
    let ok = true;
    for (const b of branches) {
      if (Array.isArray(b.enum) && b.enum.every((v) => typeof v === 'string')) {
        for (const v of b.enum) ext.push({ value: v, vn: pascal(v), unit: true, doc: b.description });
      } else if (b.properties && Array.isArray(b.required) && b.required.length === 1 && Object.keys(b.properties).length === 1) {
        const key = b.required[0];
        const vn = pascal(key);
        const body = b.properties[key];
        if (body && body.properties) {
          ext.push({ value: key, vn, fields: fieldsOf(body, `${name}${vn}`), doc: b.description });
        } else {
          const t = fieldType(body, `${name}${vn}`);
          ext.push({ value: key, vn, newtype: t.ty, doc: b.description });
        }
      } else {
        ok = false;
        break;
      }
    }
    if (ok && ext.length) {
      items.set(name, { kind: 'external', tn, variants: ext, doc });
      return;
    }
    items.set(name, { kind: 'alias', tn, ty: 'serde_json::Value', doc: (doc ? doc + '\n\n' : '') + 'Untyped: the schema is a union this generator does not model.' });
    return;
  }
  if (schema.properties || schema.type === 'object') {
    if (!schema.properties && schema.additionalProperties && schema.additionalProperties !== true) {
      const t = fieldType(schema.additionalProperties, `${name}Value`);
      items.set(name, { kind: 'alias', tn, ty: `BTreeMap<String, ${t.ty}>`, doc });
      return;
    }
    if (!schema.properties) {
      items.set(name, { kind: 'alias', tn, ty: 'serde_json::Value', doc });
      return;
    }
    const fields = fieldsOf(schema, name);
    // A struct with a union on top (McpServerElicitationRequestParams): keep
    // the common fields typed and the variant part in `rest`.
    const rest = !!(schema.oneOf || schema.anyOf);
    items.set(name, { kind: 'struct', tn, fields, rest, doc });
    return;
  }
  const t = fieldType(schema, `${name}Inner`);
  items.set(name, { kind: 'alias', tn, ty: t.ty, doc });
}

for (const r of roots) want(r);
while (queue.length) {
  const n = queue.shift();
  if (!defs[n]) {
    console.warn(`warning: no definition for ${n}`);
    items.set(n, { kind: 'alias', tn: typeName(n), ty: 'serde_json::Value', doc: 'Missing from the schema bundle.' });
    continue;
  }
  define(n, defs[n]);
}

// ── Recursion: box direct self-references ──────────────────────────────────

const byTn = new Map([...items.values()].map((it) => [it.tn, it]));
function directEdges(it) {
  const fields = it.kind === 'struct' ? it.fields : it.kind === 'tagged' || it.kind === 'external' ? it.variants.flatMap((v) => v.fields || []) : [];
  return fields.filter((f) => f.named && !/^(Vec|BTreeMap)</.test(f.ty) && !/^Option<(Vec|BTreeMap)</.test(f.ty));
}
function reaches(from, target, seen = new Set()) {
  if (from === target) return true;
  if (seen.has(from)) return false;
  seen.add(from);
  const it = byTn.get(from);
  if (!it) return false;
  if (it.kind === 'alias') {
    const m = it.ty.match(/^(?:Option<)?([A-Z][A-Za-z0-9]*)>?$/);
    return m ? reaches(m[1], target, seen) : false;
  }
  return directEdges(it).some((f) => reaches(f.named, target, seen));
}
for (const it of items.values()) {
  for (const f of directEdges(it)) {
    if (reaches(f.named, it.tn)) {
      f.ty = f.ty.startsWith('Option<') ? `Option<Box<${f.named}>>` : `Box<${f.ty}>`;
      f.boxed = true;
    }
  }
}

// ── Default-ability ────────────────────────────────────────────────────────
// A required field gets `#[serde(default)]` when its type has a `Default`, so
// a CLI that drops a field does not break the whole message. Enums default to
// their catch-all, structs derive `Default` when every field can.

const defaultMemo = new Map();
function defaultable(ty, guard = new Set()) {
  if (DEFAULTABLE.test(ty)) return true;
  const boxed = ty.match(/^Box<(.+)>$/);
  if (boxed) return defaultable(boxed[1], guard);
  const it = byTn.get(ty);
  if (!it) return false;
  if (defaultMemo.has(ty)) return defaultMemo.get(ty);
  if (guard.has(ty)) return false;
  guard.add(ty);
  let ok;
  switch (it.kind) {
    case 'strenum':
    case 'tagged':
    case 'external':
      ok = true;
      break;
    case 'alias':
      ok = defaultable(it.ty, guard);
      break;
    case 'struct':
      ok = it.fields.every((f) => f.optional || defaultable(f.ty, guard));
      break;
    default:
      ok = false;
  }
  guard.delete(ty);
  defaultMemo.set(ty, ok);
  return ok;
}

// ── Render ─────────────────────────────────────────────────────────────────

const lines = [];
const w = (s = '') => lines.push(s);

function docLines(doc, indent = '') {
  if (!doc) return;
  for (const l of String(doc).split('\n')) w(`${indent}///${l ? ' ' + l.replace(/\s+$/, '') : ''}`);
}

function renderFields(fields, indent, vis = 'pub ') {
  for (const f of fields) {
    docLines(f.doc, indent);
    const attrs = [];
    if (f.rust.replace(/^r#/, '').replace(/_$/, '') !== f.key || f.rust.endsWith('_')) attrs.push(`rename = ${JSON.stringify(f.key)}`);
    if (f.optional) attrs.push('default', 'skip_serializing_if = "Option::is_none"');
    else if (defaultable(f.ty)) attrs.push('default');
    if (attrs.length) w(`${indent}#[serde(${attrs.join(', ')})]`);
    w(`${indent}${vis}${f.rust}: ${f.ty},`);
  }
}

const sha = createHash('sha256').update(bundleText).digest('hex');

w('// @generated by scripts/codex-schema/gen.mjs — do not edit by hand.');
w(`// Source: \`codex app-server generate-json-schema\` of codex-cli ${version}`);
w(`// Bundle: codex_app_server_protocol.schemas.json, sha256 ${sha}`);
w('// Regenerate: `node scripts/codex-schema/gen.mjs` (see the script header).');
w('#![allow(dead_code, clippy::large_enum_variant, clippy::enum_variant_names, clippy::doc_lazy_continuation, clippy::too_long_first_doc_paragraph)]');
w();
w('//! Codex `app-server` protocol types, generated from the JSON Schema the CLI');
w('//! exports. Newline-delimited JSON-RPC 2.0 over stdio, without the');
w('//! `"jsonrpc"` field: `{id, method, params}` / `{id, result|error}` /');
w('//! `{method, params}`.');
w();
w('use std::collections::BTreeMap;');
w();
w('use serde::{Deserialize, Serialize};');
w();
w(`/// \`codex --version\` the schema came from.`);
w(`pub const CODEX_PROTOCOL_VERSION: &str = ${JSON.stringify(version)};`);
w(`/// sha256 of the schema bundle, to tell two generations apart.`);
w(`pub const CODEX_SCHEMA_SHA256: &str = ${JSON.stringify(sha)};`);
w();

function methodConst(m) {
  return m.replace(/[^A-Za-z0-9]+/g, '_').replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase();
}
function methodTable(title, list, modName) {
  w(`/// ${title}: \`(method, params type)\`; \`""\` when the method has no params.`);
  w(`pub mod ${modName} {`);
  const seen = new Set();
  for (const r of list) {
    let c = methodConst(r.method);
    while (seen.has(c)) c += '_';
    seen.add(c);
    w(`    pub const ${c}: &str = ${JSON.stringify(r.method)};`);
  }
  w(`    pub const ALL: &[(&str, &str)] = &[`);
  for (const r of list) w(`        (${JSON.stringify(r.method)}, ${JSON.stringify(r.params || '')}),`);
  w('    ];');
  w('}');
  w();
}
methodTable('Requests the client sends', clientRequests, 'client_request');
methodTable('Notifications the client sends', clientNotifications, 'client_notification');
methodTable('Requests the server sends (they must be answered)', serverRequests, 'server_request');
methodTable('Notifications the server sends', serverNotifications, 'server_notification');

const sorted = [...items.values()].filter((it) => it.kind !== 'pending').sort((a, b) => (a.tn < b.tn ? -1 : a.tn > b.tn ? 1 : 0));
const DERIVE = '#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]';
const DERIVE_DEFAULT = '#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]';
for (const it of sorted) {
  docLines(it.doc);
  switch (it.kind) {
    case 'alias':
      w(`pub type ${it.tn} = ${it.ty};`);
      break;
    case 'strenum': {
      w('#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]');
      w(`pub enum ${it.tn} {`);
      const used = new Set();
      for (const v of it.values) {
        let vn = pascal(v);
        while (used.has(vn) || vn === 'Unknown') vn += 'Value';
        used.add(vn);
        w(`    #[serde(rename = ${JSON.stringify(v)})]`);
        w(`    ${vn},`);
      }
      w('    /// A value this build does not know yet.');
      w('    #[default]');
      w('    #[serde(other)]');
      w('    Unknown,');
      w('}');
      break;
    }
    case 'tagged': {
      w(DERIVE_DEFAULT);
      w(`#[serde(tag = ${JSON.stringify(it.tag)})]`);
      w(`pub enum ${it.tn} {`);
      const used = new Set();
      for (const v of it.variants) {
        let vn = v.vn;
        while (used.has(vn) || vn === 'Unknown') vn += 'Value';
        used.add(vn);
        docLines(v.doc, '    ');
        w(`    #[serde(rename = ${JSON.stringify(v.value)})]`);
        if (v.fields.length === 0) {
          w(`    ${vn} {},`);
        } else {
          w(`    ${vn} {`);
          renderFields(v.fields, '        ', '');
          w('    },');
        }
      }
      w('    /// A variant this build does not know yet.');
      w('    #[default]');
      w('    #[serde(other)]');
      w('    Unknown,');
      w('}');
      break;
    }
    case 'external': {
      // Wrapped: `Known` decodes the variants of this schema, anything newer
      // lands in `Other` instead of failing the whole message.
      const kn = `${it.tn}Known`;
      w(DERIVE);
      w('#[serde(untagged)]');
      w(`pub enum ${it.tn} {`);
      w(`    Known(${kn}),`);
      w('    Other(serde_json::Value),');
      w('}');
      w();
      w(`impl Default for ${it.tn} {`);
      w('    fn default() -> Self {');
      w(`        ${it.tn}::Other(serde_json::Value::Null)`);
      w('    }');
      w('}');
      w();
      w(DERIVE);
      w(`pub enum ${kn} {`);
      const used = new Set();
      for (const v of it.variants) {
        let vn = v.vn;
        while (used.has(vn)) vn += 'Value';
        used.add(vn);
        docLines(v.doc, '    ');
        w(`    #[serde(rename = ${JSON.stringify(v.value)})]`);
        if (v.unit) w(`    ${vn},`);
        else if (v.newtype) w(`    ${vn}(${v.newtype}),`);
        else {
          w(`    ${vn} {`);
          renderFields(v.fields, '        ', '');
          w('    },');
        }
      }
      w('}');
      break;
    }
    case 'struct': {
      w(defaultable(it.tn) ? DERIVE_DEFAULT : DERIVE);
      w(`pub struct ${it.tn} {`);
      renderFields(it.fields, '    ');
      if (it.rest) {
        w('    /// The variant part of the union (`mode`-specific fields).');
        w('    #[serde(flatten)]');
        w('    pub rest: serde_json::Map<String, serde_json::Value>,');
      }
      w('}');
      break;
    }
    default:
      throw new Error(`unhandled ${it.kind}`);
  }
  w();
}

let text = lines.join('\n').replace(/\n{3,}/g, '\n\n');
if (!text.endsWith('\n')) text += '\n';

if (tmp) rmSync(tmp, { recursive: true, force: true });

if (check) {
  const cur = existsSync(out) ? readFileSync(out, 'utf8') : '';
  if (cur !== text) {
    console.error(`${out} is out of date (codex ${version})`);
    process.exit(1);
  }
  console.log(`${out} is up to date (codex ${version})`);
} else {
  writeFileSync(out, text);
  console.log(`wrote ${out}: ${items.size} types, codex ${version}`);
}
