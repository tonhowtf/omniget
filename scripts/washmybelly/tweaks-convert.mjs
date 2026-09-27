#!/usr/bin/env node
// Converts the Windows tweak sources washmybelly ships ("tweak as data",
// src-tauri/omniget-core/src/core/washmybelly/tweaks/catalog.rs):
//
// - Win11Debloat (MIT, Raphire)  Config/Features.json + Regfiles/*.reg + Regfiles/Undo/*.reg
//     -> data/washmybelly/win11debloat-features/{features.json, Regfiles/**}
//   Every feature backed by a .reg file, minus the exclusions below. The .reg
//   files are copied byte for byte (UTF-16 LE with BOM or UTF-8, as upstream
//   ships them); the Rust side parses them into typed ops. The Undo .reg is
//   what "Back to the Windows default" applies.
// - winutil (MIT, CT Tech Group)  config/tweaks.json (not strict JSON: raw
//   line breaks inside strings) -> data/washmybelly/winutil-tweaks/tweaks.json
//   Only the entries in INCLUDE_WINUTIL, and only their `registry[]` part as
//   typed ops. InvokeScript/UndoScript (PowerShell) never come along, and the
//   "Services - Set to Manual" list is not taken.
//
// Sophia (data/washmybelly/sophia-tweaks/tweaks.json) and our own items
// (data/washmybelly/tweaks/ours.json) are written by hand; this script only
// checks that they parse.
//
// Usage:
//   node scripts/washmybelly/tweaks-convert.mjs --repos <folder with the checkouts>
//   node scripts/washmybelly/tweaks-convert.mjs --repos <folder> --check   fail if a committed file differs
// Without --repos it reads $WMB_REPOS (the layout of estudos/89-washmybelly/repos).

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync, rmSync } from 'node:fs';
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
if (!repos || !existsSync(join(WD, 'Config', 'Features.json')) || !existsSync(join(WU, 'config', 'tweaks.json'))) {
  console.error('tweaks-convert: pass --repos <folder with Raphire_Win11Debloat and ChrisTitusTech_winutil>');
  process.exit(2);
}

const commitOf = (repo) => execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();

// ── Win11Debloat ────────────────────────────────────────────────────

// Left out on purpose (see SOURCE.md).
const EXCLUDE_WD = {
  DisableTelemetry: 'privacy module (privacy.sexy) owns telemetry',
  DisableLocationServices: 'privacy module owns location',
  DisableFindMyDevice: 'privacy module owns Find My Device',
  DisableBitlockerAutoEncryption: 'BitLocker stays out of washmybelly',
  DisableDeliveryOptimization: 'writes the NetworkService hive (HKU\\S-1-5-20); the winutil policy is used instead',
};
// Features implemented in PowerShell upstream that map to one typed op.
const CODE_FEATURES = {
  EnableWindowsSandbox: [{ op: 'optional_feature', name: 'Containers-DisposableClientVM', enabled: true }],
  EnableWindowsSubsystemForLinux: [
    { op: 'optional_feature', name: 'Microsoft-Windows-Subsystem-Linux', enabled: true },
    { op: 'optional_feature', name: 'VirtualMachinePlatform', enabled: true },
  ],
};
const CATEGORY_WD = {
  'Privacy & Suggested Content': 'bloat',
  System: 'system',
  'Start Menu & Search': 'taskbar_start',
  AI: 'ai',
  'Windows Update': 'updates',
  Taskbar: 'taskbar_start',
  Appearance: 'appearance',
  'File Explorer': 'explorer',
  Gaming: 'gaming',
  'Multi-tasking': 'appearance',
  'Optional Windows Features': 'features',
  Other: 'bloat',
};
const CATEGORY_OVERRIDE_WD = { DisableNotifications: 'appearance', DisableFastStartup: 'performance', DisableAnimations: 'performance', DisableTransparency: 'performance' };
const WARNING_WD = {
  DisableFastStartup: 'Startup from a full shutdown gets slower.',
  DisableNotifications: 'Hides every notification, including calendar reminders.',
  DisableWindowSnapping: 'Windows no longer snap to the screen edges.',
  DisableStartAllApps: 'Harder to find installed apps from Start.',
};

const wdRaw = JSON.parse(readFileSync(join(WD, 'Config', 'Features.json'), 'utf8').replace(/^﻿/, ''));
const groupOf = {};
const wdGroups = [];
for (const g of wdRaw.UiGroups) {
  const members = g.Values.flatMap((v) => v.FeatureIds);
  if (members.some((id) => !CODE_FEATURES[id] && !wdRaw.Features.find((f) => f.FeatureId === id)?.RegistryKey)) continue;
  const id = `w11d.${g.GroupId}`;
  for (const m of members) groupOf[m] = id;
  const def = g.Values.find((v) => /\(Default\)/.test(v.Label));
  wdGroups.push({
    id,
    label: g.Label,
    description: g.ToolTip ?? '',
    members: members.map((m) => `w11d.${m}`),
    default: def ? `w11d.${def.FeatureIds[0]}` : null,
  });
}
const regFiles = new Set();
const wdItems = [];
const wdExcluded = [];
for (const f of wdRaw.Features) {
  const id = f.FeatureId;
  if (EXCLUDE_WD[id]) {
    wdExcluded.push({ id, why: EXCLUDE_WD[id] });
    continue;
  }
  const codeOps = CODE_FEATURES[id];
  if (!f.RegistryKey && !codeOps) {
    if (f.Category) wdExcluded.push({ id, why: 'implemented in PowerShell upstream, no typed op' });
    continue;
  }
  const category = CATEGORY_OVERRIDE_WD[id] ?? CATEGORY_WD[f.Category];
  if (!category) throw new Error(`Features.json: unknown category ${f.Category} for ${id}`);
  const item = {
    id: `w11d.${id}`,
    label: f.Label,
    description: f.ToolTip ?? '',
    category,
    applies: { min_build: f.MinVersion ?? null, max_build: f.MaxVersion ?? null },
    group: groupOf[id] ?? null,
    risk: 'reversible',
    needs_restart: Boolean(f.RequiresReboot),
    source: 'win11debloat',
    license: 'MIT',
  };
  if (WARNING_WD[id]) item.warning = WARNING_WD[id];
  if (codeOps) {
    item.ops = codeOps;
  } else {
    const apply = f.RegistryKey;
    if (!existsSync(join(WD, 'Regfiles', apply))) throw new Error(`missing Regfiles/${apply}`);
    item.ops_reg = apply;
    regFiles.add(apply);
    if (f.RegistryUndoKey) {
      const undo = `Undo/${f.RegistryUndoKey}`;
      // Some undo files live next to the apply files (a group's default).
      const where = existsSync(join(WD, 'Regfiles', undo)) ? undo : f.RegistryUndoKey;
      if (!existsSync(join(WD, 'Regfiles', where))) throw new Error(`missing Regfiles/${undo}`);
      item.default_reg = where;
      regFiles.add(where);
    }
  }
  wdItems.push(item);
}

// ── winutil ─────────────────────────────────────────────────────────

/// Escape raw control characters inside JSON strings (winutil writes
/// PowerShell with literal line breaks), then parse strictly.
function tolerantJson(text) {
  let out = '';
  let inStr = false;
  let esc = false;
  for (const ch of text.replace(/^﻿/, '')) {
    if (inStr) {
      if (esc) esc = false;
      else if (ch === '\\') esc = true;
      else if (ch === '"') inStr = false;
      else if (ch === '\n') { out += '\\n'; continue; }
      else if (ch === '\r') { out += '\\r'; continue; }
      else if (ch === '\t') { out += '\\t'; continue; }
    } else if (ch === '"') inStr = true;
    out += ch;
  }
  return JSON.parse(out);
}

// Kept winutil entries: our id, category, group, and what to say. Only the
// registry part is converted; the rest is listed in SOURCE.md.
const INCLUDE_WINUTIL = {
  WPFTweaksConsumerFeatures: { id: 'winutil.consumer_features', category: 'bloat' },
  WPFTweaksEdgeDebloat: { id: 'winutil.edge_debloat', category: 'bloat', warning: 'Edge will say it is managed by your organization.' },
  WPFTweaksDeliveryOptimization: { id: 'winutil.delivery_optimization', category: 'updates' },
  WPFTweaksUTC: { id: 'winutil.utc_clock', category: 'system', warning: 'Only for computers that also boot Linux.' },
  WPFTweaksWPBT: { id: 'winutil.wpbt', category: 'system', warning: 'Stops anti-theft and driver tools your PC maker installs at boot.' },
  WPFTweaksPreventDeviceMetadataFromNetwork: { id: 'winutil.device_metadata', category: 'bloat' },
  WPFTweaksDisableBGapps: { id: 'winutil.background_apps', category: 'performance', warning: 'Store apps (the new Teams too) stop notifying you in the background.' },
  WPFTweaksIPv46: { id: 'winutil.ipv4_preferred', category: 'network', group: 'winutil.ipv6' },
  WPFTweaksTeredo: { id: 'winutil.teredo_off', category: 'network', group: 'winutil.ipv6', warning: 'Some games and Xbox party chat use Teredo.' },
  WPFTweaksDisableIPv6: { id: 'winutil.ipv6_off', category: 'network', group: 'winutil.ipv6', warning: 'Networks that only offer IPv6 stop working.' },
  WPFToggleDetailedBSoD: { id: 'winutil.bsod_details', category: 'system' },
  WPFToggleBatteryPercentage: { id: 'winutil.battery_percentage', category: 'taskbar_start' },
  WPFToggleVerboseLogon: { id: 'winutil.verbose_logon', category: 'system' },
  WPFToggleScrollbars: { id: 'winutil.scrollbars_visible', category: 'appearance' },
  WPFToggleGameMode: { id: 'winutil.game_mode', category: 'gaming' },
  WPFToggleLongPaths: { id: 'winutil.long_paths', category: 'system' },
  WPFToggleLoginBlur: { id: 'winutil.logon_blur', category: 'appearance' },
  WPFToggleDisableLockscreen: { id: 'winutil.no_lock_screen', category: 'appearance' },
  WPFMultiplaneOverlay: { id: 'winutil.mpo', category: 'gaming' },
};
const GROUPS_WINUTIL = [
  { id: 'winutil.ipv6', label: 'IPv6', description: 'The three write the same value: only one can be on.', members: ['winutil.ipv4_preferred', 'winutil.teredo_off', 'winutil.ipv6_off'], default: null },
  { id: 'winutil.mpo', label: 'Multiplane Overlay', description: 'Turn it off only if you see flicker or black screens.', members: ['winutil.mpo.enabled', 'winutil.mpo.compat', 'winutil.mpo.off'], default: 'winutil.mpo.enabled' },
];
const KIND = { DWord: 'REG_DWORD', QWord: 'REG_QWORD', String: 'REG_SZ', ExpandString: 'REG_EXPAND_SZ', Binary: 'REG_BINARY', MultiString: 'REG_MULTI_SZ' };
const REMOVE = '<RemoveEntry>';

function splitPath(p) {
  const m = /^(HKLM|HKCU|HKU):\\(.+)$/.exec(p);
  if (!m) throw new Error(`winutil: unexpected path ${p}`);
  return { hive: m[1], key: m[2].replace(/\\+$/, '') };
}

function regOp(r, value) {
  const { hive, key } = splitPath(r.Path);
  if (value === REMOVE) return { op: 'reg_delete', hive, key, name: r.Name };
  const kind = KIND[r.Type];
  if (!kind) throw new Error(`winutil: unknown type ${r.Type} at ${r.Path}`);
  return { op: 'reg_set', hive, key, name: r.Name, kind, data: String(value) };
}

const wuRaw = tolerantJson(readFileSync(join(WU, 'config', 'tweaks.json'), 'utf8'));
const wuItems = [];
const wuExcluded = [];
for (const [wid, v] of Object.entries(wuRaw)) {
  const inc = INCLUDE_WINUTIL[wid];
  if (!inc) {
    wuExcluded.push({ id: wid, title: v.Content });
    continue;
  }
  const base = {
    label: v.Content,
    description: v.Description ?? '',
    category: inc.category,
    applies: { min_build: null, max_build: null },
    risk: 'reversible',
    needs_restart: false,
    source: 'winutil',
    license: 'MIT',
  };
  if (inc.warning) base.warning = inc.warning;
  if (v.Type === 'Combobox') {
    // One item per choice, as a group (winutil's Multiplane Overlay).
    const choices = v.ComboItems.split('|');
    const slug = { Enabled: 'enabled', 'Disabled (Compatibility)': 'compat', 'Fully Disabled': 'off' };
    for (const c of choices) {
      if (!slug[c]) throw new Error(`winutil: unknown choice ${c} in ${wid}`);
      wuItems.push({
        ...base,
        id: `${inc.id}.${slug[c]}`,
        label: `${v.Content}: ${c}`,
        description: v.ComboDescriptions?.[c] ?? base.description,
        group: inc.id,
        ops: v.registry.map((r) => regOp(r, r.Values[c])),
      });
    }
    continue;
  }
  const ops = (v.registry ?? []).map((r) => regOp(r, r.Value));
  if (!ops.length) throw new Error(`winutil: ${wid} has no registry part`);
  const item = { ...base, id: inc.id, group: inc.group ?? null, ops };
  const defaults = (v.registry ?? []).filter((r) => r.OriginalValue !== undefined).map((r) => regOp(r, r.OriginalValue));
  if (defaults.length === ops.length) item.default_ops = defaults;
  // A toggle's DefaultState says what an absent value means (winutil
  // Get-WinUtilToggleStatus): "true" = the tweak's own value.
  (v.registry ?? []).forEach((r, i) => {
    if (ops[i].op !== 'reg_set') return;
    if (r.DefaultState === 'true') ops[i].absent_means = ops[i].data;
    else if (r.DefaultState === 'false' && r.OriginalValue !== undefined && r.OriginalValue !== REMOVE) ops[i].absent_means = String(r.OriginalValue);
  });
  if (v.InvokeScript?.length) item.note = 'registry part only; the upstream PowerShell step is not run';
  wuItems.push(item);
}
for (const id of Object.keys(INCLUDE_WINUTIL)) {
  if (!wuRaw[id]) throw new Error(`winutil: ${id} no longer exists upstream`);
}

// ── Hand-written sources: they must parse ───────────────────────────
for (const p of [join(DATA, 'sophia-tweaks', 'tweaks.json'), join(DATA, 'tweaks', 'ours.json')]) {
  const t = JSON.parse(readFileSync(p, 'utf8'));
  if (!Array.isArray(t.items) || !t.items.length) throw new Error(`${p}: no items`);
}

// ── Write ───────────────────────────────────────────────────────────
const outputs = [
  {
    dir: 'win11debloat-features',
    file: 'features.json',
    license: join(WD, 'LICENSE'),
    body: {
      source: 'https://github.com/Raphire/Win11Debloat',
      commit: commitOf(WD),
      license: 'MIT',
      generated_by: 'scripts/washmybelly/tweaks-convert.mjs',
      upstream_file: 'Config/Features.json',
      count: wdItems.length,
      excluded: wdExcluded,
      groups: wdGroups,
      items: wdItems,
    },
  },
  {
    dir: 'winutil-tweaks',
    file: 'tweaks.json',
    license: join(WU, 'LICENSE'),
    body: {
      source: 'https://github.com/ChrisTitusTech/winutil',
      commit: commitOf(WU),
      license: 'MIT',
      generated_by: 'scripts/washmybelly/tweaks-convert.mjs',
      upstream_file: 'config/tweaks.json',
      count: wuItems.length,
      excluded: wuExcluded,
      groups: GROUPS_WINUTIL,
      items: wuItems,
    },
  },
];

// The Rust table of embedded .reg files (include_bytes! needs literal paths).
const RS = join(ROOT, 'src-tauri', 'omniget-core', 'src', 'core', 'washmybelly', 'tweaks', 'regfiles.rs');
const rsText =
  '//! Generated by scripts/washmybelly/tweaks-convert.mjs: do not edit.\n' +
  '//! The Win11Debloat .reg files the catalog uses, byte for byte.\n\n' +
  'pub static REG_FILES: &[(&str, &[u8])] = &[\n' +
  [...regFiles].sort().map((f) => `    (${JSON.stringify(f)}, include_bytes!(${JSON.stringify(`../../../../data/washmybelly/win11debloat-features/Regfiles/${f}`)})),\n`).join('') +
  '];\n';
outputs.push({ path: RS, text: rsText });

let stale = false;
const regDir = join(DATA, 'win11debloat-features', 'Regfiles');
for (const o of outputs) {
  if (o.text !== undefined) {
    if (check) {
      if ((existsSync(o.path) ? readFileSync(o.path, 'utf8') : '') !== o.text) {
        console.error(`tweaks-convert: ${o.path} is out of date; run without --check`);
        stale = true;
      }
    } else {
      writeFileSync(o.path, o.text);
      console.log('tweaks-convert: wrote tweaks/regfiles.rs');
    }
    continue;
  }
  const dir = join(DATA, o.dir);
  const path = join(dir, o.file);
  const text = JSON.stringify(o.body, null, 1) + '\n';
  if (check) {
    const current = existsSync(path) ? readFileSync(path, 'utf8') : '';
    if (current !== text) {
      console.error(`tweaks-convert: ${path} is out of date; run without --check`);
      stale = true;
    }
  } else {
    mkdirSync(dir, { recursive: true });
    writeFileSync(path, text);
    copyFileSync(o.license, join(dir, 'LICENSE'));
    console.log(`tweaks-convert: wrote ${o.dir}/${o.file} (${o.body.count}) from ${o.body.commit.slice(0, 12)}`);
  }
}
if (check) {
  for (const f of regFiles) {
    const a = readFileSync(join(WD, 'Regfiles', f));
    const b = existsSync(join(regDir, f)) ? readFileSync(join(regDir, f)) : Buffer.alloc(0);
    if (!a.equals(b)) {
      console.error(`tweaks-convert: Regfiles/${f} differs from upstream`);
      stale = true;
    }
  }
  if (stale) process.exit(1);
  console.log('tweaks-convert: up to date');
} else {
  rmSync(regDir, { recursive: true, force: true });
  for (const f of regFiles) {
    mkdirSync(dirname(join(regDir, f)), { recursive: true });
    copyFileSync(join(WD, 'Regfiles', f), join(regDir, f));
  }
  console.log(`tweaks-convert: copied ${regFiles.size} .reg files`);
}
