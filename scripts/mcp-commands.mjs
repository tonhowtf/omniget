#!/usr/bin/env node
// Writes src-tauri/src/mcp/command_catalog.json: every command in handlers.rs, with its area and
// arguments as the frontend passes them. omniget_commands serves this list and omniget_invoke calls it.
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri");
const handlers = readFileSync(join(root, "src/handlers.rs"), "utf8");
const entries = [...handlers.matchAll(/^\s*((?:[a-z_0-9]+::)+)([a-z_0-9]+),?\s*$/gm)].map((m) => ({
  path: m[1].split("::").filter(Boolean).filter((s) => s !== "crate"),
  name: m[2],
}));

function walk(dir, out = []) {
  for (const f of readdirSync(dir)) {
    if (f === "target" || f === "gen") continue;
    const p = join(dir, f);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (f.endsWith(".rs")) out.push(p);
  }
  return out;
}
const sources = walk(join(root, "src")).concat(walk(join(root, "omniget-core/src"))).map((p) => ({ p, t: readFileSync(p, "utf8") }));

const INJECTED = /^&?\s*(?:tauri::)?(?:ipc::)?(?:AppHandle|Window|WebviewWindow|Webview|State|Request)\b/;
const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

function params(text, open) {
  let depth = 0, angle = 0, cur = "", out = [];
  for (let i = open + 1; i < text.length; i++) {
    const c = text[i];
    if (c === "(" || c === "[") depth++;
    if (c === "<") angle++;
    if (c === ">" && text[i - 1] !== "-") angle--;
    if (c === ")" && depth === 0) { if (cur.trim()) out.push(cur.trim()); return out; }
    if (c === ")" || c === "]") depth--;
    if (c === "," && depth === 0 && angle === 0) { out.push(cur.trim()); cur = ""; continue; }
    cur += c;
  }
  return out;
}

function find(entry) {
  const re = new RegExp(`#\\[tauri::command([^\\]]*)\\]\\s*(?:#\\[[^\\]]*\\]\\s*)*pub\\s+(?:async\\s+)?fn\\s+${entry.name}\\b(?:<[^>]*>)?\\s*\\(`);
  const hits = sources.map((s) => ({ ...s, m: re.exec(s.t) })).filter((h) => h.m);
  hits.sort((a, b) => score(b.p, entry) - score(a.p, entry));
  return hits[0];
}
const score = (p, e) => e.path.filter((s) => p.includes(`/${s}`)).length;

const missing = [];
const commands = entries.map((e) => {
  const area = e.path[0] === "commands" ? e.path[1] : e.path[0];
  const hit = find(e);
  if (!hit) { missing.push(e.name); return { name: e.name, area, args: [] }; }
  const snake = /rename_all\s*=\s*"snake_case"/.test(hit.m[1]);
  const open = hit.m.index + hit.m[0].length - 1;
  const args = [];
  let channel = false;
  for (const raw of params(hit.t, open)) {
    const colon = raw.indexOf(":");
    if (colon < 0) continue;
    const name = raw.slice(0, colon).replace(/^mut\s+/, "").trim();
    const type = raw.slice(colon + 1).replace(/\s+/g, " ").trim();
    if (INJECTED.test(type)) continue;
    if (/\bChannel</.test(type)) channel = true;
    args.push({ name: snake ? name : camel(name), type, ...(type.startsWith("Option<") ? { optional: true } : {}) });
  }
  const before = hit.t.slice(0, hit.m.index).split("\n");
  const doc = [];
  for (let i = before.length - 2; i >= 0 && /^\s*\/\/\//.test(before[i]); i--) doc.unshift(before[i].replace(/^\s*\/\/\/\s?/, ""));
  return { name: e.name, area, args, ...(doc.length ? { doc: doc.join(" ").slice(0, 300) } : {}), ...(channel ? { channel: true } : {}) };
});
commands.sort((a, b) => a.area.localeCompare(b.area) || a.name.localeCompare(b.name));
writeFileSync(join(root, "src/mcp/command_catalog.json"), JSON.stringify(commands) + "\n");
console.log(`${commands.length} commands, ${new Set(commands.map((c) => c.area)).size} areas, ${commands.filter((c) => c.channel).length} with channels`);
if (missing.length) console.log("no signature found:", missing.join(" "));
