#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..', '..');
const OUT_DIR = join(ROOT, 'src-tauri', 'omniget-core', 'data', 'washmybelly', 'kudu');
const OUT = join(OUT_DIR, 'rules.json');

const args = process.argv.slice(2);
const check = args.includes('--check');
const repoArg = args.indexOf('--repo');
const repo = repoArg >= 0 ? args[repoArg + 1] : process.env.KUDU_REPO;
if (!repo || !existsSync(join(repo, 'rules', 'win32'))) {
  console.error('kudu-convert: pass --repo <path to a Kudu checkout> (or set KUDU_REPO)');
  process.exit(2);
}

const commit = execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const read = (f) => JSON.parse(readFileSync(join(repo, 'rules', 'win32', f), 'utf8'));

const VARS = {
  HOME: '%USERPROFILE%',
  LOCALAPPDATA: '%LOCALAPPDATA%',
  APPDATA: '%APPDATA%',
  WINDIR: '%WINDIR%',
  PROGRAMDATA: '%ProgramData%',
  PROGRAMFILES: '%ProgramFiles%',
  PROGRAMFILES_X86: '%ProgramFiles(x86)%',
  SYSTEMDRIVE: '%SystemDrive%',
  TMPDIR: '%TEMP%',
};

function win(path) {
  const out = path.replace(/\$\{([A-Z_0-9]+)\}/g, (_, v) => {
    if (!VARS[v]) throw new Error(`unknown Kudu variable \${${v}} in ${path}`);
    return VARS[v];
  });
  if (!out.startsWith('%')) throw new Error(`target without a root variable: ${path}`);
  return out.replace(/\//g, '\\');
}

const PROTECTED = [
  '%windir%/system32',
  '%windir%/syswow64',
  '%windir%/winsxs',
  '%windir%/prefetch',
  '%windir%/installer',
  '%windir%/appcompat',
  '%windir%/boot',
  '%windir%/servicing',
  '%programdata%/microsoft/windows defender',
  '%systemdrive%/windows.old',
  '%systemdrive%/$windows.~bt',
  '%systemdrive%/recovery',
  '%userprofile%/documents',
  '%userprofile%/desktop',
  '%userprofile%/pictures',
  '%userprofile%/music',
  '%userprofile%/videos',
  '%userprofile%/onedrive',
  '%appdata%/microsoft/credentials',
  '%localappdata%/microsoft/credentials',
  '%appdata%/microsoft/protect',
];

function protectedHit(target) {
  const t = target.replace(/\\/g, '/').toLowerCase();
  return PROTECTED.find((p) => t === p || t.startsWith(p + '/')) || null;
}

const skipped = [];
const rules = [];

function slug(s) {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '');
}

const DEV = new Set(
  'vscode jetbrains jetbrains-logs npm yarn pnpm uv bun pip pipenv nuget sccache cargo go go-build gradle maven composer docker postman github-desktop android-studio conda unity visual-studio ruby-gems deno node-gyp electron-builder electron java-cache aws-cli gcloud typescript-typings vscode-cpptools poetry zed vscodium vscode-insiders sublime-text windsurf cursor cursor-partitions scoop chocolatey-logs winget-logs unreal-ddc hyper github-copilot-logs claude-code-cache codex-cache codex-temp t3-code t3-code-partitions t3-code-logs todesktop-builder'.split(' '),
);

const BROWSER_EXE = {
  chrome: 'chrome.exe', edge: 'msedge.exe', brave: 'brave.exe', opera: 'opera.exe', operaGX: 'opera.exe',
  vivaldi: 'vivaldi.exe', arc: 'Arc.exe', chromium: 'chrome.exe', thorium: 'thorium.exe',
  supermium: 'supermium.exe', helium: 'helium.exe', cromite: 'chrome.exe', catsxp: 'catsxp.exe',
  firefox: 'firefox.exe', librewolf: 'librewolf.exe', waterfox: 'waterfox.exe', floorp: 'floorp.exe', zen: 'zen.exe',
};

function rule({ id, category, label, description = '', targets, minAge, action, admin = false, running = [], cacheReset = false }) {
  const kept = [];
  for (const t of targets) {
    const hit = protectedHit(t);
    if (hit) skipped.push({ id, target: t, reason: `inside protected ${hit}` });
    else kept.push(t);
  }
  if (!kept.length) return;
  const needsAdmin = admin || kept.some((t) => /^%WINDIR%/i.test(t));
  let desc = description;
  if (cacheReset) desc = (desc ? desc + ' ' : '') + 'Performance cache: the next launch is slower while it rebuilds.';
  rules.push({
    id,
    os: ['windows'],
    category,
    label_key: null,
    label,
    description: desc,
    targets: kept,
    include: [],
    exclude: [],
    running,
    min_age_days: minAge,
    action,
    destination: 'unlink',
    risk: 'destructive',
    requires: needsAdmin ? [{ type: 'admin' }] : [],
    source: 'Kudu',
    license: 'MIT',
  });
}

const DELETE_CONTENTS = { type: 'delete_contents' };
const DELETE = { type: 'delete' };

const SYSTEM_SKIP = {
  '${WINDIR}/Prefetch': 'Prefetch is on the placebo list (slower boot and launches until it rebuilds)',
  '${WINDIR}/WinSxS': 'component store cleanup is DISM territory (maintenance module), never ResetBase',
  '${SYSTEMDRIVE}/Windows.old': 'rollback of the previous Windows; cleanmgr territory, not a cache',
  '${WINDIR}/Installer/$PatchCache$': 'MSI patch baseline: needed to uninstall updates',
  '${WINDIR}/AppCompat/Programs': 'holds Amcache.hve (application compatibility), not a cache',
};
const sys = read('system.json');
for (const t of sys.cleanTargets) {
  const id = `kudu.system.${slug(t.subcategory)}`;
  if (SYSTEM_SKIP[t.path]) {
    skipped.push({ id, target: t.path, reason: SYSTEM_SKIP[t.path] });
    continue;
  }
  if (t.cleanupAction) {
    skipped.push({ id, target: t.path, reason: `native action ${t.cleanupAction} is implemented by rules_windows (maintenance)` });
    continue;
  }
  const target = t.childSubdir ? `${win(t.path)}\\*\\${t.childSubdir}` : win(t.path);
  rule({
    id,
    category: /log|trac|diagnos|report|dump/i.test(t.subcategory) ? 'logs' : 'system',
    label: t.subcategory,
    description: t.description || '',
    targets: [target],
    minAge: t.deepRecencyCheck ? 2 : 1,
    action: DELETE_CONTENTS,
    admin: !!t.needsAdmin,
    cacheReset: !!t.cacheReset,
  });
}
for (const t of sys.singleFileTargets || []) {
  rule({
    id: `kudu.system.${slug(t.subcategory)}`,
    category: 'logs',
    label: t.subcategory,
    description: t.description || '',
    targets: [win(t.path)],
    minAge: 1,
    action: DELETE,
  });
}

const NATIVE = {
  'pnpm-prune': { type: 'command', program: 'pnpm', args: ['store', 'prune'] },
};
const APP_SKIP = {
  'steam-shader-cache': 'wipes the shader cache of installed games too; rules_windows offers only uninstalled games',
};
const NATIVE_SKIP = {
  'docker-build-prune': 'rules_windows ships its own docker builder prune rule',
  'uv-prune': 'uv is not in plan::NATIVE_PROGRAMS',
  'windows-components': 'component store cleanup is DISM territory',
  'delivery-optimization': 'rules_windows ships its own Delivery Optimization rule',
};

function convertApps(file, prefix, defaultCategory) {
  for (const a of read(file).apps) {
    const id = `kudu.${prefix}.${a.id.replace(/-/g, '_')}`;
    const category = defaultCategory === 'apps' && DEV.has(a.id) ? 'dev' : defaultCategory;
    const minAge = a.minAgeDays || 1;
    if (APP_SKIP[a.id]) {
      skipped.push({ id, target: a.paths.join(', '), reason: APP_SKIP[a.id] });
      continue;
    }
    if (a.cleanupAction) {
      if (NATIVE_SKIP[a.cleanupAction]) {
        skipped.push({ id, target: a.cleanupAction, reason: NATIVE_SKIP[a.cleanupAction] });
        continue;
      }
      const cmd = NATIVE[a.cleanupAction];
      rules.push({
        id, os: ['windows'], category, label_key: null, label: a.name, description: a.description || '',
        targets: [], include: [], exclude: [], running: [], min_age_days: 0,
        action: { ...cmd, measure: a.paths.map(win) },
        destination: 'unlink', risk: 'destructive',
        requires: [{ type: 'program', name: cmd.program }],
        source: 'Kudu', license: 'MIT',
      });
      continue;
    }
    let targets;
    let action = DELETE_CONTENTS;
    let exclude = [];
    if (a.childSubdir) {
      targets = a.paths.map((p) => `${win(p)}\\*\\${a.childSubdir}`);
    } else if (a.recursiveMatch) {
      const m = a.recursiveMatch;
      const anchors = m.anchorPaths ? m.anchorPaths.map((ap) => ap.replace(/\//g, '\\')) : [''];
      targets = [];
      for (const p of a.paths) {
        for (const anchor of anchors) {
          const base = anchor ? `${win(p)}\\${anchor}` : win(p);
          for (const t of m.targets) {
            targets.push(`${base}\\*\\${t}`);
            if (m.anchorPaths) targets.push(`${base}\\${t}`);
          }
        }
      }
      exclude = m.excludedAncestors || [];
    } else if (a.fileMatch) {
      const f = a.fileMatch;
      if (f.skipIfChildExists && f.skipIfChildExists.length) {
        skipped.push({ id, target: a.paths.join(', '), reason: 'fileMatch.skipIfChildExists has no equivalent in the Rule shape' });
        continue;
      }
      targets = [];
      for (const p of a.paths) {
        for (const n of f.names) targets.push(f.childDirSuffix ? `${win(p)}\\*${f.childDirSuffix}\\${n}` : `${win(p)}\\${n}`);
      }
      action = DELETE;
      rule({ id, category, label: a.name, description: a.description || '', targets, minAge: f.minAgeDays, action });
      continue;
    } else {
      targets = a.paths.map(win);
    }
    const before = rules.length;
    rule({ id, category, label: a.name, description: a.description || '', targets, minAge, action, cacheReset: !!a.cacheReset });
    if (rules.length > before) rules[rules.length - 1].exclude = exclude;
  }
}
convertApps('apps.json', 'apps', 'apps');
convertApps('gaming.json', 'games', 'games');
convertApps('gpu-cache.json', 'shader', 'shader_cache');

const br = read('browsers.json');
for (const b of br.chromium) {
  const base = win(b.base);
  const targets = [];
  for (const d of br.chromiumCacheDirs.profile) {
    const dir = d.dir.replace(/\//g, '\\');
    targets.push(`${base}\\*\\${dir}`);
    if (b.key.startsWith('opera')) targets.push(`${base}\\${dir}`);
  }
  for (const d of br.chromiumCacheDirs.shared || []) targets.push(`${base}\\${d.dir.replace(/\//g, '\\')}`);
  rule({
    id: `kudu.browser.${slug(b.key)}.cache`,
    category: 'browser',
    label: `${b.key} cache`,
    description: 'Web, code, GPU and shader caches; rebuilt on demand.',
    targets,
    minAge: 1,
    action: DELETE_CONTENTS,
    running: BROWSER_EXE[b.key] ? [BROWSER_EXE[b.key]] : [],
  });
}
for (const [key, cfg] of [['firefox', br.firefox], ...(br.firefoxForks || []).map((f) => [f.key, f])]) {
  rule({
    id: `kudu.browser.${key}.cache`,
    category: 'browser',
    label: `${key} cache`,
    description: 'Per-profile web content cache; rebuilt on demand.',
    targets: [`${win(cfg.cache)}\\*\\cache2`],
    minAge: 1,
    action: DELETE_CONTENTS,
    running: BROWSER_EXE[key] ? [BROWSER_EXE[key]] : [],
  });
}

skipped.push({ id: 'kudu.databases', target: 'databases.json', reason: 'history/cookie databases: BleachBit and rules_windows own browser privacy' });
skipped.push({ id: 'kudu.steam', target: 'steam.json', reason: 'redistributable folders live inside game folders; never game files' });
skipped.push({ id: 'kudu.misc', target: 'misc.json', reason: 'event logs and Recycle Bin are not cleanup targets here' });

const ids = new Set();
for (const r of rules) {
  if (ids.has(r.id)) throw new Error(`duplicate id ${r.id}`);
  ids.add(r.id);
}

const out = {
  source: 'https://github.com/AdventDevInc/kudu',
  commit,
  license: 'MIT',
  generated_by: 'scripts/washmybelly/kudu-convert.mjs',
  count: rules.length,
  skipped,
  rules,
};
const text = JSON.stringify(out, null, 1) + '\n';

if (check) {
  const current = existsSync(OUT) ? readFileSync(OUT, 'utf8') : '';
  if (current !== text) {
    console.error(`kudu-convert: ${OUT} is out of date; run without --check`);
    process.exit(1);
  }
  console.log(`kudu-convert: up to date (${rules.length} rules, ${skipped.length} skipped)`);
} else {
  mkdirSync(OUT_DIR, { recursive: true });
  writeFileSync(OUT, text);
  copyFileSync(join(repo, 'LICENSE'), join(OUT_DIR, 'LICENSE'));
  console.log(`kudu-convert: wrote ${rules.length} rules (${skipped.length} skipped) from ${commit.slice(0, 12)}`);
}
