export const CHROME_WEB_STORE_DETAIL = "https://chromewebstore.google.com/detail/";

export const CHROME_WEB_STORE_UPDATE_URL = "https://clients2.google.com/service/update2/crx";

export const MV3_ONLY_PERMISSIONS = [
  "declarativeNetRequest",
  "declarativeNetRequestWithHostAccess",
  "declarativeNetRequestFeedback",
  "offscreen",
  "readingList",
  "scripting",
  "sidePanel",
  "userScripts",
];

export const MV2_ONLY_PERMISSIONS = ["webRequestBlocking"];

const MV3_SET = new Set(MV3_ONLY_PERMISSIONS);
const MV2_SET = new Set(MV2_ONLY_PERMISSIONS);

const NON_EXTENSION_TYPES = new Set([
  "theme",
  "hosted_app",
  "packaged_app",
  "legacy_packaged_app",
]);

export function storeUrlFor(id) {
  const clean = typeof id === "string" ? id.trim() : "";
  if (!clean) return null;
  return `${CHROME_WEB_STORE_DETAIL}${clean}`;
}

export function largestIcon(icons) {
  if (!Array.isArray(icons) || icons.length === 0) return null;
  let best = null;
  for (const icon of icons) {
    if (!icon || typeof icon.url !== "string" || !icon.url) continue;
    const size = Number(icon.size) || 0;
    if (!best || size > best.size) best = { size, url: icon.url };
  }
  return best ? best.url : null;
}

function sortedStrings(list) {
  if (!Array.isArray(list)) return [];
  return list
    .filter((item) => typeof item === "string" && item.length > 0)
    .slice()
    .sort((a, b) => a.localeCompare(b));
}

export function detectManifestVersion(info) {
  const permissions = Array.isArray(info?.permissions) ? info.permissions : [];

  const mv3Hit = permissions.find((p) => MV3_SET.has(p));
  if (mv3Hit) {
    return {
      version: 3,
      source: `MV3-only permission "${mv3Hit}"`,
      confidence: "likely",
    };
  }

  const mv2Hit = permissions.find((p) => MV2_SET.has(p));
  if (mv2Hit) {
    return {
      version: 2,
      source: `MV2-only permission "${mv2Hit}"`,
      confidence: "likely",
    };
  }

  if (NON_EXTENSION_TYPES.has(info?.type)) {
    return {
      version: null,
      source: `not applicable to a ${info.type}`,
      confidence: "unknown",
    };
  }

  return {
    version: null,
    source: "no permission tells the two apart",
    confidence: "unknown",
  };
}

export function detectStoreOrigin(info) {
  const installType = info?.installType ?? null;
  if (installType === "development") return false;
  if (typeof info?.updateUrl === "string" && info.updateUrl.startsWith(CHROME_WEB_STORE_UPDATE_URL)) {
    return true;
  }
  if (installType === "sideload" || installType === "other") return false;
  if (installType === "normal") return true;
  return null;
}

export function normalizeExtension(info) {
  const id = typeof info?.id === "string" ? info.id : "";
  const manifest = detectManifestVersion(info);
  const overridden = Number(info?.manifestVersion);
  const hasOverride = overridden === 2 || overridden === 3;

  return {
    id,
    name: typeof info?.name === "string" ? info.name : id,
    shortName: typeof info?.shortName === "string" ? info.shortName : null,
    version: typeof info?.version === "string" ? info.version : null,
    versionName: typeof info?.versionName === "string" ? info.versionName : null,
    description: typeof info?.description === "string" ? info.description.trim() : "",
    enabled: Boolean(info?.enabled),
    disabledReason: info?.disabledReason ?? null,
    mayDisable: info?.mayDisable === undefined ? null : Boolean(info.mayDisable),
    type: info?.type ?? null,
    installType: info?.installType ?? null,
    offlineEnabled: info?.offlineEnabled === undefined ? null : Boolean(info.offlineEnabled),
    manifestVersion: hasOverride ? overridden : manifest.version,
    manifestVersionSource: hasOverride ? "read from the extension manifest" : manifest.source,
    manifestVersionConfidence: hasOverride ? "certain" : manifest.confidence,
    permissions: sortedStrings(info?.permissions),
    hostPermissions: sortedStrings(info?.hostPermissions),
    icon: largestIcon(info?.icons),
    homepageUrl: typeof info?.homepageUrl === "string" ? info.homepageUrl : null,
    updateUrl: typeof info?.updateUrl === "string" ? info.updateUrl : null,
    optionsUrl: typeof info?.optionsUrl === "string" && info.optionsUrl ? info.optionsUrl : null,
    storeUrl: storeUrlFor(id),
    fromWebStore: detectStoreOrigin(info),
  };
}

function groupWeight(entry) {
  if (entry.manifestVersion === 2) return 0;
  if (entry.manifestVersion === null) return 1;
  return 2;
}

export function compareExtensions(a, b) {
  const byGroup = groupWeight(a) - groupWeight(b);
  if (byGroup !== 0) return byGroup;
  const nameA = (a.name || a.id || "").toLocaleLowerCase();
  const nameB = (b.name || b.id || "").toLocaleLowerCase();
  const byName = nameA.localeCompare(nameB);
  if (byName !== 0) return byName;
  return (a.id || "").localeCompare(b.id || "");
}

export function summarize(entries) {
  const byType = {};
  const byInstallType = {};
  let mv2 = 0;
  let mv3 = 0;
  let unknownManifest = 0;
  let disabled = 0;

  for (const entry of entries) {
    if (entry.manifestVersion === 2) mv2++;
    else if (entry.manifestVersion === 3) mv3++;
    else unknownManifest++;
    if (!entry.enabled) disabled++;
    const type = entry.type || "unknown";
    byType[type] = (byType[type] || 0) + 1;
    const install = entry.installType || "unknown";
    byInstallType[install] = (byInstallType[install] || 0) + 1;
  }

  return {
    total: entries.length,
    mv2,
    mv3,
    unknownManifest,
    disabled,
    enabled: entries.length - disabled,
    sideloaded: byInstallType.sideload || 0,
    development: byInstallType.development || 0,
    admin: byInstallType.admin || 0,
    byType,
    byInstallType,
  };
}

export function buildBackup(list, { generatedAt = new Date().toISOString(), browser = "Chrome" } = {}) {
  const entries = (Array.isArray(list) ? list : []).map(normalizeExtension).sort(compareExtensions);

  return {
    format: "omniget-extension-inventory",
    formatVersion: 1,
    browser,
    generatedAt,
    storeListingChecked: false,
    summary: summarize(entries),
    groups: {
      mv2: entries.filter((e) => e.manifestVersion === 2).map((e) => e.id),
      unknown: entries.filter((e) => e.manifestVersion === null).map((e) => e.id),
      mv3: entries.filter((e) => e.manifestVersion === 3).map((e) => e.id),
    },
    extensions: entries,
  };
}

export function buildJson(backup) {
  return `${JSON.stringify(backup, null, 2)}\n`;
}

export function backupFileName(extension, generatedAt) {
  const iso = typeof generatedAt === "string" ? generatedAt : new Date().toISOString();
  const day = iso.slice(0, 10) || "export";
  return `omniget-extensions-${day}.${extension}`;
}

function permissionsLine(entry) {
  const all = [...entry.permissions, ...entry.hostPermissions];
  return all.length > 0 ? all.join(", ") : "none";
}

function originLabel(entry) {
  if (entry.fromWebStore === true) return "Chrome Web Store";
  if (entry.fromWebStore === false) return "outside the store";
  return "unknown origin";
}

function stateLabel(entry) {
  const parts = [entry.enabled ? "enabled" : "disabled"];
  if (!entry.enabled && entry.disabledReason) parts.push(`(${entry.disabledReason})`);
  if (entry.installType) parts.push(`· ${entry.installType}`);
  return parts.join(" ");
}

function manifestLabel(entry) {
  if (entry.manifestVersion === null) return `Manifest version unknown — ${entry.manifestVersionSource}`;
  const suffix = entry.manifestVersionConfidence === "certain" ? "" : " (inferred)";
  return `Manifest V${entry.manifestVersion}${suffix} — ${entry.manifestVersionSource}`;
}

const MV2_WARNING =
  "Chrome is turning off Manifest V2 extensions. The ones below are the ones to replace first.";

const UNKNOWN_NOTE =
  "The management API does not report the manifest version, so these could not be classified from permissions alone. Open the store page to check.";

const STORE_NOTE =
  "Store links were not checked against the network, so an extension already removed from the store still shows a link here.";

function mdEscape(text) {
  return String(text ?? "").replace(/([\\`*_[\]|])/g, "\\$1").replace(/\r?\n/g, " ");
}

function markdownEntry(entry) {
  const lines = [];
  lines.push(`### ${mdEscape(entry.name)}${entry.version ? ` — ${mdEscape(entry.version)}` : ""}`);
  lines.push("");
  lines.push(`- ID: \`${entry.id}\``);
  lines.push(`- Store page: ${entry.storeUrl ?? "unknown"}`);
  lines.push(`- State: ${stateLabel(entry)}, ${originLabel(entry)}`);
  lines.push(`- ${manifestLabel(entry)}`);
  if (entry.description) lines.push(`- Description: ${mdEscape(entry.description)}`);
  if (entry.homepageUrl) lines.push(`- Homepage: ${entry.homepageUrl}`);
  lines.push(`- Permissions: ${mdEscape(permissionsLine(entry))}`);
  lines.push("");
  return lines.join("\n");
}

export function buildMarkdown(backup) {
  const { summary } = backup;
  const out = [];
  out.push(`# Installed extensions — ${backup.browser}`);
  out.push("");
  out.push(`Exported by OmniGet on ${backup.generatedAt}.`);
  out.push("");
  out.push(`- Total: ${summary.total}`);
  out.push(`- Manifest V2 (at risk): ${summary.mv2}`);
  out.push(`- Manifest V3: ${summary.mv3}`);
  out.push(`- Manifest version unknown: ${summary.unknownManifest}`);
  out.push(`- Disabled: ${summary.disabled}`);
  out.push(`- Sideloaded: ${summary.sideloaded}`);
  out.push(`- Unpacked (development): ${summary.development}`);
  out.push("");
  out.push(STORE_NOTE);
  out.push("");

  const mv2 = backup.extensions.filter((e) => e.manifestVersion === 2);
  out.push("## Manifest V2 — at risk");
  out.push("");
  out.push(MV2_WARNING);
  out.push("");
  if (mv2.length === 0) {
    out.push("Nothing found in this group.");
    out.push("");
  } else {
    for (const entry of mv2) out.push(markdownEntry(entry));
  }

  const unknown = backup.extensions.filter((e) => e.manifestVersion === null);
  out.push("## Manifest version unknown");
  out.push("");
  out.push(UNKNOWN_NOTE);
  out.push("");
  if (unknown.length === 0) {
    out.push("Nothing found in this group.");
    out.push("");
  } else {
    for (const entry of unknown) out.push(markdownEntry(entry));
  }

  const mv3 = backup.extensions.filter((e) => e.manifestVersion === 3);
  out.push("## Manifest V3");
  out.push("");
  if (mv3.length === 0) {
    out.push("Nothing found in this group.");
    out.push("");
  } else {
    out.push("| Extension | Version | State | Store page |");
    out.push("| --- | --- | --- | --- |");
    for (const entry of mv3) {
      out.push(
        `| ${mdEscape(entry.name)} | ${mdEscape(entry.version ?? "")} | ${stateLabel(entry)} | ${entry.storeUrl ?? ""} |`,
      );
    }
    out.push("");
  }

  return `${out.join("\n").trimEnd()}\n`;
}

export function escapeHtml(text) {
  return String(text ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function htmlEntry(entry) {
  const rows = [];
  rows.push(`<h3>${escapeHtml(entry.name)}${entry.version ? ` <span class="version">${escapeHtml(entry.version)}</span>` : ""}</h3>`);
  if (entry.description) rows.push(`<p class="desc">${escapeHtml(entry.description)}</p>`);
  rows.push("<dl>");
  rows.push(`<dt>ID</dt><dd><code>${escapeHtml(entry.id)}</code></dd>`);
  if (entry.storeUrl) {
    rows.push(
      `<dt>Store page</dt><dd><a href="${escapeHtml(entry.storeUrl)}" rel="noreferrer">${escapeHtml(entry.storeUrl)}</a></dd>`,
    );
  }
  rows.push(`<dt>State</dt><dd>${escapeHtml(stateLabel(entry))}, ${escapeHtml(originLabel(entry))}</dd>`);
  rows.push(`<dt>Manifest</dt><dd>${escapeHtml(manifestLabel(entry))}</dd>`);
  if (entry.homepageUrl) {
    rows.push(
      `<dt>Homepage</dt><dd><a href="${escapeHtml(entry.homepageUrl)}" rel="noreferrer">${escapeHtml(entry.homepageUrl)}</a></dd>`,
    );
  }
  rows.push(`<dt>Permissions</dt><dd class="perms">${escapeHtml(permissionsLine(entry))}</dd>`);
  rows.push("</dl>");
  return `<article class="ext">${rows.join("")}</article>`;
}

export function buildHtml(backup) {
  const { summary } = backup;
  const mv2 = backup.extensions.filter((e) => e.manifestVersion === 2);
  const unknown = backup.extensions.filter((e) => e.manifestVersion === null);
  const mv3 = backup.extensions.filter((e) => e.manifestVersion === 3);

  const mv3Rows = mv3
    .map(
      (entry) =>
        `<tr><td>${escapeHtml(entry.name)}</td><td>${escapeHtml(entry.version ?? "")}</td><td>${escapeHtml(stateLabel(entry))}</td><td>${
          entry.storeUrl
            ? `<a href="${escapeHtml(entry.storeUrl)}" rel="noreferrer">store</a>`
            : ""
        }</td></tr>`,
    )
    .join("");

  const empty = '<p class="empty">Nothing found in this group.</p>';

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Installed extensions — ${escapeHtml(backup.browser)}</title>
<style>
:root { color-scheme: dark; }
body { margin: 0; padding: 32px 20px; background: #101114; color: #f5f7fb;
  font-family: "Segoe UI", system-ui, sans-serif; line-height: 1.55; }
main { max-width: 880px; margin: 0 auto; }
h1 { font-size: 26px; margin: 0 0 6px; }
h2 { font-size: 19px; margin: 34px 0 10px; }
h3 { font-size: 15px; margin: 0 0 6px; }
.meta { color: #95a0b7; font-size: 13px; margin: 0 0 20px; }
.summary { display: flex; flex-wrap: wrap; gap: 10px; margin: 0 0 18px; padding: 0; list-style: none; }
.summary li { background: rgba(255,255,255,.05); border: 1px solid rgba(255,255,255,.08);
  border-radius: 10px; padding: 8px 12px; font-size: 13px; }
.summary b { display: block; font-size: 18px; }
.note { color: #95a0b7; font-size: 13px; }
.warn { color: #ffb27a; }
.ext { border: 1px solid rgba(255,255,255,.08); border-radius: 12px; padding: 14px 16px; margin: 0 0 10px;
  background: rgba(18,20,26,.9); }
.at-risk .ext { border-color: rgba(255,125,56,.45); }
.version { color: #95a0b7; font-weight: 400; font-size: 13px; }
.desc { margin: 0 0 8px; color: #d9deea; font-size: 13px; }
dl { display: grid; grid-template-columns: 130px 1fr; gap: 4px 12px; margin: 0; font-size: 13px; }
dt { color: #95a0b7; }
dd { margin: 0; overflow-wrap: anywhere; }
.perms { color: #d9deea; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
a { color: #ff9a66; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 7px 8px; border-bottom: 1px solid rgba(255,255,255,.08); }
th { color: #95a0b7; font-weight: 500; }
.empty { color: #95a0b7; font-size: 13px; }
</style>
</head>
<body>
<main>
<h1>Installed extensions — ${escapeHtml(backup.browser)}</h1>
<p class="meta">Exported by OmniGet on ${escapeHtml(backup.generatedAt)}.</p>
<ul class="summary">
<li><b>${summary.total}</b>total</li>
<li><b>${summary.mv2}</b>Manifest V2</li>
<li><b>${summary.mv3}</b>Manifest V3</li>
<li><b>${summary.unknownManifest}</b>unknown</li>
<li><b>${summary.disabled}</b>disabled</li>
<li><b>${summary.sideloaded}</b>sideloaded</li>
<li><b>${summary.development}</b>unpacked</li>
</ul>
<p class="note">${escapeHtml(STORE_NOTE)}</p>
<section class="at-risk">
<h2 class="warn">Manifest V2 — at risk</h2>
<p class="note">${escapeHtml(MV2_WARNING)}</p>
${mv2.length === 0 ? empty : mv2.map(htmlEntry).join("")}
</section>
<section>
<h2>Manifest version unknown</h2>
<p class="note">${escapeHtml(UNKNOWN_NOTE)}</p>
${unknown.length === 0 ? empty : unknown.map(htmlEntry).join("")}
</section>
<section>
<h2>Manifest V3</h2>
${
    mv3.length === 0
      ? empty
      : `<table><thead><tr><th>Extension</th><th>Version</th><th>State</th><th>Store page</th></tr></thead><tbody>${mv3Rows}</tbody></table>`
  }
</section>
</main>
</body>
</html>
`;
}

export async function probeManifestVersion(id, { fetchImpl = globalThis.fetch } = {}) {
  if (!id || typeof fetchImpl !== "function") return null;
  try {
    const response = await fetchImpl(`chrome-extension://${id}/manifest.json`);
    if (!response?.ok) return null;
    const manifest = await response.json();
    const version = Number(manifest?.manifest_version);
    return version === 2 || version === 3 ? version : null;
  } catch {
    return null;
  }
}

export async function withProbedManifestVersions(list, options = {}) {
  const items = Array.isArray(list) ? list : [];
  const probed = await Promise.all(items.map((item) => probeManifestVersion(item?.id, options)));
  return items.map((item, index) =>
    probed[index] === null ? item : { ...item, manifestVersion: probed[index] },
  );
}
