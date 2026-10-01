#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..', '..');
const DATA = join(ROOT, 'src-tauri', 'omniget-core', 'data', 'washmybelly');

const args = process.argv.slice(2);
const check = args.includes('--check');
const reposArg = args.indexOf('--repos');
const repos = reposArg >= 0 ? args[reposArg + 1] : process.env.WMB_REPOS;
const WD = repos && join(repos, 'Raphire_Win11Debloat');
const WU = repos && join(repos, 'ChrisTitusTech_winutil');
const SO = repos && join(repos, 'farag2_Sophia-Script-for-Windows');
if (!repos || !existsSync(join(WD, 'Config', 'Apps.json')) || !existsSync(join(WU, 'config', 'appx.json')) || !existsSync(join(SO, 'src'))) {
  console.error('win-apps-convert: pass --repos <folder with Raphire_Win11Debloat, ChrisTitusTech_winutil, farag2_Sophia-Script-for-Windows>');
  process.exit(2);
}

const commitOf = (repo) => execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const readJson = (p) => JSON.parse(readFileSync(p, 'utf8').replace(/^﻿/, ''));

const wdRaw = readJson(join(WD, 'Config', 'Apps.json'));
const RECOMMENDATION = { safe: 'safe', optional: 'optional', unsafe: 'dangerous' };
const wdApps = wdRaw.Apps.map((a) => {
  const ids = Array.isArray(a.AppId) ? a.AppId : [a.AppId];
  const recommendation = RECOMMENDATION[a.Recommendation];
  if (!recommendation) throw new Error(`Apps.json: unknown recommendation ${a.Recommendation} for ${a.FriendlyName}`);
  const removal = String(a.RemovalMethod).toLowerCase();
  if (removal !== 'appx' && removal !== 'winget') throw new Error(`Apps.json: unknown removal ${a.RemovalMethod}`);
  return {
    ids,
    name: a.FriendlyName,
    description: a.Description ?? '',
    recommendation,
    removal,
    default: Boolean(a.SelectedByDefault),
  };
});

const wuRaw = readJson(join(WU, 'config', 'appx.json'));
const storeIds = {};
for (const v of Object.values(wuRaw)) {
  if (v.PackageId && v.StoreId) storeIds[v.PackageId] = v.StoreId;
}

const never = new Set();
const sophiaFiles = [];
for (const edition of readdirSync(join(SO, 'src')).sort()) {
  const psm = join(SO, 'src', edition, 'Module', 'Sophia.psm1');
  if (!existsSync(psm)) continue;
  const text = readFileSync(psm, 'utf8');
  const re = /\$ExcludedAppxPackages\s*=\s*@\(([\s\S]*?)\n\s*\)/g;
  let m;
  let found = false;
  while ((m = re.exec(text))) {
    found = true;
    for (const line of m[1].split('\n')) {
      const s = line.trim();
      if (!s || s.startsWith('#')) continue;
      const q = s.match(/^"([^"]+)"/);
      if (q) never.add(q[1]);
    }
  }
  if (found) sophiaFiles.push(`src/${edition}/Module/Sophia.psm1`);
}
if (never.size < 10) throw new Error(`Sophia: only ${never.size} excluded packages found, the format changed`);

const outputs = [
  {
    dir: 'win11debloat',
    file: 'apps.json',
    license: join(WD, 'LICENSE'),
    body: {
      source: 'https://github.com/Raphire/Win11Debloat',
      commit: commitOf(WD),
      license: 'MIT',
      generated_by: 'scripts/washmybelly/win-apps-convert.mjs',
      upstream_file: 'Config/Apps.json',
      upstream_version: wdRaw.Version ?? null,
      count: wdApps.length,
      apps: wdApps,
    },
  },
  {
    dir: 'winutil',
    file: 'appx.json',
    license: join(WU, 'LICENSE'),
    body: {
      source: 'https://github.com/ChrisTitusTech/winutil',
      commit: commitOf(WU),
      license: 'MIT',
      generated_by: 'scripts/washmybelly/win-apps-convert.mjs',
      upstream_file: 'config/appx.json',
      count: Object.keys(storeIds).length,
      store_ids: Object.fromEntries(Object.entries(storeIds).sort(([a], [b]) => a.localeCompare(b))),
    },
  },
  {
    dir: 'sophia',
    file: 'excluded_appx.json',
    license: join(SO, 'LICENSE'),
    body: {
      source: 'https://github.com/farag2/Sophia-Script-for-Windows',
      commit: commitOf(SO),
      license: 'MIT',
      generated_by: 'scripts/washmybelly/win-apps-convert.mjs',
      upstream_files: sophiaFiles,
      count: never.size,
      never: [...never].sort((a, b) => a.localeCompare(b)),
    },
  },
];

let stale = false;
for (const o of outputs) {
  const dir = join(DATA, o.dir);
  const path = join(dir, o.file);
  const text = JSON.stringify(o.body, null, 1) + '\n';
  if (check) {
    const current = existsSync(path) ? readFileSync(path, 'utf8') : '';
    if (current !== text) {
      console.error(`win-apps-convert: ${path} is out of date; run without --check`);
      stale = true;
    }
  } else {
    mkdirSync(dir, { recursive: true });
    writeFileSync(path, text);
    copyFileSync(o.license, join(dir, 'LICENSE'));
    console.log(`win-apps-convert: wrote ${o.dir}/${o.file} (${o.body.count}) from ${o.body.commit.slice(0, 12)}`);
  }
}
if (check) {
  if (stale) process.exit(1);
  console.log('win-apps-convert: up to date');
}
