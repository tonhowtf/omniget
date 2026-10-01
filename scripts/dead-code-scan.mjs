#!/usr/bin/env node

import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join, relative, dirname, extname, basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const asJson = args.includes("--json");
const onlyArg = args.find((a) => a.startsWith("--only"));
const only = onlyArg ? (onlyArg.split("=")[1] ?? args[args.indexOf(onlyArg) + 1] ?? "").split(",") : null;
const want = (name) => !only || only.includes(name);

const STALE_TERMS = [
  /\budemy\b/i,
  /\bhotmart\b/i,
  /\brocketseat\b/i,
  /\bkiwify\b/i,
  /\bcourses?\b/i,
  /\bplugins?\b/i,
  /\bmarketplace\b/i,
  /\bstudy\b/i,
  /\banki\b/i,
  /\bflashcards?\b/i,
  /\btelegram\b/i,
  /Tools\s*(→|>|›)/,
  /\bTools section\b/i,
  /\b156 tools\b/i,
];
const STALE_ALLOW = [/platform_optout\.rs$/, /bilibili[\\/]parser[\\/]cheese\.rs$/, /PLATFORM-OWNERS\.md$/, /dead-code-scan\.mjs$/, /MEDIA-KIT\.md$/, /llms\.txt$/, /\.test\.(ts|mjs)$/, /_test\.rs$/, /claude-plugin\//];

const SKIP_DIRS = new Set(["node_modules", "target", ".svelte-kit", "build", ".git", "gen", "binaries", ".pnpm-store", "docs", "estudos", ".agents", ".claude", "art"]);

function walk(dir, exts, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(e.name) || e.name.startsWith(".DS_")) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, exts, out);
    else if (!exts || exts.includes(extname(e.name))) out.push(p);
  }
  return out;
}
const read = (p) => readFileSync(p, "utf8");
const rel = (p) => relative(ROOT, p);
const lineOf = (text, index) => text.slice(0, index).split("\n").length;

const FRONT_EXT = [".svelte", ".ts", ".js", ".mjs"];
const frontFiles = walk(join(ROOT, "src"), FRONT_EXT);
const rustFiles = walk(join(ROOT, "src-tauri"), [".rs"]);
const extFiles = walk(join(ROOT, "browser-extension", "chrome"), [".js", ".html", ".json"]);
const frontText = new Map(frontFiles.map((f) => [f, read(f)]));
const rustText = new Map(rustFiles.map((f) => [f, read(f)]));
const allFront = [...frontText.values()].join("\n");
const allRust = [...rustText.values()].join("\n");
const report = {};

if (want("commands")) {
  const lib = read(join(ROOT, "src-tauri", "src", "handlers.rs"));
  const block = lib.slice(lib.indexOf("generate_handler!["));
  const body = block.slice(0, block.indexOf("])"));
  const registered = body
    .slice(body.indexOf("[") + 1)
    .replace(/\/\/[^\n]*/g, "")
    .replace(/#\[[^\]]*\]/g, "")
    .split(",")
    .map((e) => e.trim().split("::").pop().trim())
    .filter((n) => /^[a-z][a-z0-9_]*$/.test(n));
  const regSet = new Set(registered);
  const invoked = new Map();
  for (const [f, t] of frontText) {
    for (const m of t.matchAll(/invoke\s*(?:<[^()]*?>)?\s*\(\s*["'`]([a-z0-9_]+)["'`]/g)) {
      if (!invoked.has(m[1])) invoked.set(m[1], `${rel(f)}:${lineOf(t, m.index)}`);
    }
  }
  const quoted = new Set([...allFront.matchAll(/["'`]([a-z][a-z0-9_]+)["'`]/g)].map((m) => m[1]));
  report.commands = {
    registeredNeverInvoked: registered.filter((n) => !invoked.has(n) && !quoted.has(n)),
    invokedNotRegistered: [...invoked].filter(([n]) => !regSet.has(n)).map(([n, at]) => `${n}  (${at})`),
  };
}

if (want("events")) {
  const emitted = new Map();
  for (const [f, t] of rustText) {
    for (const m of t.matchAll(/\.emit(?:_to|_filter)?\s*\(\s*(?:[^,()]+,\s*)?"([^"]+)"/g)) {
      if (!emitted.has(m[1])) emitted.set(m[1], `${rel(f)}:${lineOf(t, m.index)}`);
    }
  }
  for (const [f, t] of frontText) {
    for (const m of t.matchAll(/\bemit(?:To)?\s*\(\s*["'`]([^"'`]+)["'`]/g)) {
      if (!emitted.has(m[1])) emitted.set(m[1], `${rel(f)}:${lineOf(t, m.index)}`);
    }
  }
  const listened = new Map();
  for (const [f, t] of frontText) {
    for (const m of t.matchAll(/\b(?:listen|once)\s*(?:<[^()]*?>)?\s*\(\s*["'`]([^"'`]+)["'`]/g)) {
      if (!listened.has(m[1])) listened.set(m[1], `${rel(f)}:${lineOf(t, m.index)}`);
    }
  }
  for (const [f, t] of rustText) {
    for (const m of t.matchAll(/\.(?:listen|once|listen_any)\s*\(\s*"([^"]+)"/g)) {
      if (!listened.has(m[1])) listened.set(m[1], `${rel(f)}:${lineOf(t, m.index)}`);
    }
  }
  report.events = {
    emittedNeverListened: [...emitted].filter(([n]) => !listened.has(n) && !allFront.includes(n)).map(([n, at]) => `${n}  (${at})`),
    listenedNeverEmitted: [...listened].filter(([n]) => !emitted.has(n) && !allRust.includes(`"${n}"`)).map(([n, at]) => `${n}  (${at})`),
  };
}

if (want("i18n")) {
  const en = JSON.parse(read(join(ROOT, "src", "lib", "i18n", "en.json")));
  const keys = [];
  const flatten = (o, p = "") => {
    for (const [k, v] of Object.entries(o)) {
      const key = p ? `${p}.${k}` : k;
      if (v && typeof v === "object") flatten(v, key);
      else keys.push(key);
    }
  };
  flatten(en);
  const i18nDir = join("src", "lib", "i18n");
  const frontNoI18n = [...frontText].filter(([f]) => !f.includes(i18nDir) || /\.test\.ts$/.test(f)).map(([, t]) => t).join("\n");
  const haystack = frontNoI18n + "\n" + allRust + "\n" + read(join(ROOT, "src", "app.html"));
  const dynamic = new Set();
  for (const m of haystack.matchAll(/["'`]([a-z0-9_]+(?:\.[a-z0-9_]+)*\.?[a-z0-9_]*)(?:\$\{|["'`]\s*\+)/gi)) if (m[1].includes(".")) dynamic.add(m[1]);
  for (const m of haystack.matchAll(/pluralKey\(\s*["'`]([^"'`]+)["'`]/g)) dynamic.add(`${m[1]}_`);
  const unused = keys.filter((k) => {
    if (k.startsWith("keys.")) return false;
    if (haystack.includes(k)) return false;
    for (const prefix of dynamic) if (k.startsWith(prefix)) return false;
    return true;
  });
  report.i18n = { total: keys.length, unreferenced: unused };
}

if (want("modules")) {
  const importers = new Map(frontFiles.map((f) => [f, 0]));
  const aliases = { $lib: join(ROOT, "src", "lib"), $components: join(ROOT, "src", "components") };
  const resolveSpec = (from, spec) => {
    let base;
    for (const [a, p] of Object.entries(aliases)) if (spec === a || spec.startsWith(a + "/")) base = join(p, spec.slice(a.length));
    if (!base && spec.startsWith(".")) base = resolve(dirname(from), spec);
    if (!base) return null;
    for (const c of [base, `${base}.ts`, `${base}.js`, `${base}.svelte`, `${base}.svelte.ts`, join(base, "index.ts"), join(base, "index.js")]) {
      if (importers.has(c)) return c;
    }
    return null;
  };
  for (const [f, t] of frontText) {
    for (const m of t.matchAll(/(?:import|export)\s[^"'`]*?from\s*["'`]([^"'`]+)["'`]|import\s*\(\s*["'`]([^"'`]+)["'`]\s*\)|import\s*["'`]([^"'`]+)["'`]/g)) {
      const target = resolveSpec(f, m[1] ?? m[2] ?? m[3]);
      if (target && target !== f) importers.set(target, importers.get(target) + 1);
    }
  }
  const isEntry = (f) => {
    const b = basename(f);
    return b.startsWith("+") || b.startsWith("hooks.") || /\.test\.(ts|js)$/.test(b) || b === "app.d.ts" || f.includes(`${join("src", "lib", "i18n")}`);
  };
  report.modules = { importedByNothing: frontFiles.filter((f) => importers.get(f) === 0 && !isEntry(f)).map(rel) };
}

if (want("rust")) {
  const counts = new Map();
  for (const m of allRust.matchAll(/\b([a-z_][a-z0-9_]{2,})\b/g)) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  const commandAttr = /#\[tauri::command[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([a-z0-9_]+)/g;
  const commands = new Set([...allRust.matchAll(commandAttr)].map((m) => m[1]));
  const hits = [];
  for (const [f, t] of rustText) {
    if (/[\\/]tests?[\\/]|_test\.rs$|[\\/]benches[\\/]/.test(f)) continue;
    for (const m of t.matchAll(/^\s*pub(?:\([^)]*\))?\s+(?:const\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)/gm)) {
      const name = m[1];
      if (name.length < 4 || commands.has(name) || name === "main" || name === "new" || name === "default") continue;
      if ((counts.get(name) ?? 0) <= 1) hits.push(`${name}  (${rel(f)}:${lineOf(t, m.index)})`);
    }
  }
  report.rust = { pubFnNoCaller: hits };
}

if (want("exports")) {
  const counts = new Map();
  for (const t of frontText.values()) for (const m of t.matchAll(/\b([A-Za-z_$][A-Za-z0-9_$]{2,})\b/g)) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  const hits = [];
  for (const [f, t] of frontText) {
    if (!f.endsWith(".ts") || /\.test\.ts$|\.d\.ts$/.test(f) || f.includes(join("lib", "i18n"))) continue;
    for (const m of t.matchAll(/^export\s+(?:async\s+)?(?:function\*?|const|let|class|type|interface|enum)\s+([A-Za-z_$][A-Za-z0-9_$]*)/gm)) {
      if ((counts.get(m[1]) ?? 0) <= 1) hits.push(`${m[1]}  (${rel(f)}:${lineOf(t, m.index)})`);
    }
  }
  report.exports = { noOtherUse: hits };
}

if (want("static")) {
  const staticDir = join(ROOT, "static");
  const assets = walk(staticDir, null);
  const css = walk(join(ROOT, "src"), [".css"]).map(read).join("\n") + walk(staticDir, [".json", ".css", ".html"]).map(read).join("\n");
  const refs = allFront + allRust + css + read(join(ROOT, "src", "app.html")) + read(join(ROOT, "src-tauri", "tauri.conf.json"));
  const templated = [...refs.matchAll(/["'`](\/[a-z0-9_\/-]+\/)[^"'`]*\$\{/gi)].map((m) => m[1]);
  report.static = {
    unreferenced: assets
      .filter((a) => {
        const name = basename(a);
        if (name.startsWith(".")) return false;
        const webPath = "/" + relative(staticDir, a).split("\\").join("/");
        if (templated.some((dir) => webPath.startsWith(dir))) return false;
        return !refs.includes(webPath) && !refs.includes(name) && !refs.includes(name.replace(extname(name), ""));
      })
      .map((a) => `${rel(a)}  (${Math.round(statSync(a).size / 1024)} KB)`),
  };
}

if (want("deps")) {
  const pkg = JSON.parse(read(join(ROOT, "package.json")));
  const configs = ["vite.config.js", "svelte.config.js", "vitest.config.ts", "tsconfig.json"].map((f) => (existsSync(join(ROOT, f)) ? read(join(ROOT, f)) : "")).join("\n");
  const scripts = walk(join(ROOT, "scripts"), [".js", ".mjs", ".ts"]).map(read).join("\n");
  const jsHay = allFront + configs + scripts + JSON.stringify(pkg.scripts ?? {});
  const npmUnused = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies }).filter((d) => {
    if (d.startsWith("@types/")) return !jsHay.includes(d.slice(7));
    if (d === "@tauri-apps/cli") return false;
    return !jsHay.includes(`"${d}"`) && !jsHay.includes(`'${d}'`) && !jsHay.includes(`${d}/`) && !jsHay.includes(`"${d}`) && !jsHay.includes(` ${d} `) && !jsHay.includes(`${d}"`);
  });
  const crateUnused = [];
  const manifests = walk(join(ROOT, "src-tauri"), [".toml"]).filter((f) => basename(f) === "Cargo.toml");
  for (const man of manifests) {
    const toml = read(man);
    const src = walk(dirname(man), [".rs"]).map(read).join("\n");
    const sections = toml.split(/^\[/m).filter((s) => /^(target\.[^\]]+\.)?(dev-|build-)?dependencies\]/.test(s));
    for (const s of sections) {
      for (const m of s.matchAll(/^([A-Za-z0-9_-]+)\s*=/gm)) {
        const name = m[1];
        const pkgRename = s.slice(m.index).split("\n")[0].match(/package\s*=\s*"([^"]+)"/);
        const LIB_NAMES = { "md-5": "md5", "sha-1": "sha1" };
        const ident = LIB_NAMES[name] ?? name.replace(/-/g, "_");
        if (!new RegExp(`\\b${ident}\\b`).test(src) && !pkgRename) crateUnused.push(`${name}  (${rel(man)})`);
      }
    }
  }
  report.deps = { npmUnused, crateUnused };
}

if (want("stale")) {
  const hits = [];
  const en = read(join(ROOT, "src", "lib", "i18n", "en.json"));
  en.split("\n").forEach((line, i) => {
    const m = line.match(/^\s*"([^"]+)":\s*"(.*)",?\s*$/);
    if (m && !m[1].includes("cheese") && STALE_TERMS.some((r) => r.test(m[2]))) hits.push(`src/lib/i18n/en.json:${i + 1}  ${m[1]}: ${m[2].slice(0, 120)}`);
  });
  const stringFiles = [...rustFiles, ...frontFiles.filter((f) => !f.includes(join("lib", "i18n"))), ...extFiles];
  for (const f of stringFiles) {
    if (STALE_ALLOW.some((r) => r.test(f))) continue;
    const t = f.endsWith(".rs") ? rustText.get(f) : read(f);
    const lits = f.endsWith(".rs") ? t.matchAll(/"((?:[^"\\\n]|\\.){8,})"/g) : t.matchAll(/(["'`])((?:(?!\1)[^\\\n]|\\.){8,})\1|>([^<>{}\n]{8,})</g);
    for (const m of lits) {
      const s = m[2] ?? m[1] ?? m[3];
      if (s && /\s/.test(s) && STALE_TERMS.some((r) => r.test(s))) hits.push(`${rel(f)}:${lineOf(t, m.index)}  ${s.slice(0, 120)}`);
    }
  }
  report.stale = hits;
}

if (asJson) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const print = (title, list) => {
    if (!list) return;
    console.log(`\n## ${title} (${list.length})`);
    for (const x of list) console.log(`  ${x}`);
  };
  print("Commands registered but never invoked from the frontend", report.commands?.registeredNeverInvoked);
  print("Commands invoked but not registered (broken calls)", report.commands?.invokedNotRegistered);
  print("Events emitted but never listened to", report.events?.emittedNeverListened);
  print("Events listened to but never emitted", report.events?.listenedNeverEmitted);
  if (report.i18n) print(`i18n keys no file references (of ${report.i18n.total})`, report.i18n.unreferenced);
  print("Frontend modules nothing imports", report.modules?.importedByNothing);
  print("Rust pub fn named nowhere else", report.rust?.pubFnNoCaller);
  print("TypeScript exports named nowhere else", report.exports?.noOtherUse);
  print("static/ files nothing references", report.static?.unreferenced);
  print("npm dependencies no file uses", report.deps?.npmUnused);
  print("Crate dependencies no source file names", report.deps?.crateUnused);
  print("Texts that still name removed features", report.stale);
  console.log("\nCandidates only. Check each one before deleting it.");
}
