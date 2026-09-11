/**
 * Página "Back up your extension list".
 *
 * Lê `chrome.management.getAll()` e transforma a lista em três arquivos:
 * JSON (completo, para reimportar), HTML (leitura, com as MV2 no topo) e
 * Markdown. Nada sai da máquina.
 *
 * A permissão `management` é opcional de propósito: ela não vem declarada em
 * `permissions`, então a extensão não pede acesso à lista de extensões de
 * ninguém que não clicar no botão. `chrome.permissions.request` só funciona a
 * partir de um gesto do usuário — daí o botão "Allow access and scan".
 */

import {
  backupFileName,
  buildBackup,
  buildHtml,
  buildJson,
  buildMarkdown,
  withProbedManifestVersions,
} from "../src/ext-backup.js";

const MANAGEMENT = { permissions: ["management"] };

const permissionEl = document.getElementById("permission-status");
const permissionMessage = document.getElementById("permission-message");
const grantBtn = document.getElementById("grant");
const rescanBtn = document.getElementById("rescan");
const revokeBtn = document.getElementById("revoke");
const jsonBtn = document.getElementById("export-json");
const htmlBtn = document.getElementById("export-html");
const mdBtn = document.getElementById("export-md");
const statusEl = document.getElementById("status");
const summaryEl = document.getElementById("summary");
const resultsEl = document.getElementById("results");
const mv2ListEl = document.getElementById("mv2-list");

let backup = null;

function setPermissionState(state, message) {
  permissionEl.classList.remove("found", "missing");
  if (state === "found") permissionEl.classList.add("found");
  if (state === "missing") permissionEl.classList.add("missing");
  permissionMessage.textContent = message;
}

function setStatus(message, kind) {
  statusEl.textContent = message ?? "";
  statusEl.classList.remove("ok", "error");
  if (kind === "ok") statusEl.classList.add("ok");
  if (kind === "error") statusEl.classList.add("error");
}

function setExportsEnabled(enabled) {
  jsonBtn.disabled = !enabled;
  htmlBtn.disabled = !enabled;
  mdBtn.disabled = !enabled;
}

async function hasPermission() {
  try {
    return await chrome.permissions.contains(MANAGEMENT);
  } catch {
    return false;
  }
}

/** `chrome.management.getAll()` com promessa, tolerando o callback antigo. */
function getAllExtensions() {
  return new Promise((resolve, reject) => {
    if (!chrome.management?.getAll) {
      reject(new Error("The management API is not available in this browser."));
      return;
    }
    try {
      chrome.management.getAll((list) => {
        const error = chrome.runtime?.lastError;
        if (error) reject(new Error(error.message));
        else resolve(list || []);
      });
    } catch (error) {
      reject(error instanceof Error ? error : new Error(String(error)));
    }
  });
}

function renderSummary(summary) {
  const cards = [
    { label: "installed", value: summary.total, risk: false },
    { label: "Manifest V2", value: summary.mv2, risk: summary.mv2 > 0 },
    { label: "Manifest V3", value: summary.mv3, risk: false },
    { label: "unknown", value: summary.unknownManifest, risk: false },
    { label: "disabled", value: summary.disabled, risk: false },
    { label: "sideloaded", value: summary.sideloaded, risk: false },
    { label: "unpacked", value: summary.development, risk: false },
  ];

  summaryEl.textContent = "";
  for (const card of cards) {
    const li = document.createElement("li");
    if (card.risk) li.classList.add("risk");
    const strong = document.createElement("b");
    strong.textContent = String(card.value);
    li.append(strong, document.createTextNode(card.label));
    summaryEl.append(li);
  }
  summaryEl.hidden = false;
}

function renderMv2(entries) {
  mv2ListEl.textContent = "";

  if (entries.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent =
      "No Manifest V2 extension was identified. The export still lists everything you have installed.";
    mv2ListEl.append(empty);
    resultsEl.hidden = false;
    return;
  }

  for (const entry of entries) {
    const row = document.createElement("div");
    row.className = "ext-row";

    if (entry.icon) {
      const img = document.createElement("img");
      img.src = entry.icon;
      img.alt = "";
      row.append(img);
    }

    const body = document.createElement("div");
    body.className = "ext-body";

    const name = document.createElement("div");
    name.className = "ext-name";
    name.textContent = entry.version ? `${entry.name} — ${entry.version}` : entry.name;

    const meta = document.createElement("div");
    meta.className = "ext-meta";
    meta.textContent = `${entry.enabled ? "enabled" : "disabled"} · ${entry.installType || "unknown"} · `;
    if (entry.storeUrl) {
      const link = document.createElement("a");
      link.href = entry.storeUrl;
      link.target = "_blank";
      link.rel = "noreferrer";
      link.textContent = "store page";
      meta.append(link);
    } else {
      meta.append(document.createTextNode(entry.id));
    }

    const perms = document.createElement("div");
    perms.className = "ext-perms";
    const all = [...entry.permissions, ...entry.hostPermissions];
    perms.textContent = `Permissions: ${all.length > 0 ? all.join(", ") : "none"}`;

    body.append(name, meta, perms);
    row.append(body);
    mv2ListEl.append(row);
  }

  resultsEl.hidden = false;
}

function saveFile(content, fileName, mime) {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  document.body.append(link);
  link.click();
  link.remove();
  // Um tick é suficiente: o download já pegou o blob.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

async function scan() {
  setStatus("Reading the installed extensions…");
  let raw;
  try {
    raw = await getAllExtensions();
  } catch (error) {
    setStatus(`Could not read the extension list: ${error?.message ?? error}`, "error");
    return;
  }

  // Tenta o manifest de verdade quando a outra extensão o deixa legível;
  // o resto cai na inferência por permissão.
  const enriched = await withProbedManifestVersions(raw);
  backup = buildBackup(enriched, { browser: "Chrome" });

  renderSummary(backup.summary);
  renderMv2(backup.extensions.filter((entry) => entry.manifestVersion === 2));
  setExportsEnabled(true);
  rescanBtn.hidden = false;
  revokeBtn.hidden = false;

  const risky = backup.summary.mv2;
  setStatus(
    risky > 0
      ? `${backup.summary.total} extensions read — ${risky} still on Manifest V2. Export a copy before Chrome turns them off.`
      : `${backup.summary.total} extensions read. Export a copy to keep the list.`,
    "ok",
  );
}

async function requestPermission() {
  let granted = false;
  try {
    granted = await chrome.permissions.request(MANAGEMENT);
  } catch (error) {
    setStatus(`The browser refused the request: ${error?.message ?? error}`, "error");
    return;
  }

  if (!granted) {
    setPermissionState("missing", "Access to the extension list was denied.");
    setStatus(
      "Without that access the list cannot be read. Nothing else changed — click the button again if you change your mind.",
      "error",
    );
    return;
  }

  setPermissionState("found", "Access granted — reading the list on this machine only.");
  grantBtn.hidden = true;
  await scan();
}

async function revokePermission() {
  try {
    await chrome.permissions.remove(MANAGEMENT);
  } catch (error) {
    setStatus(`Could not drop the permission: ${error?.message ?? error}`, "error");
    return;
  }
  backup = null;
  setExportsEnabled(false);
  summaryEl.hidden = true;
  resultsEl.hidden = true;
  rescanBtn.hidden = true;
  revokeBtn.hidden = true;
  grantBtn.hidden = false;
  setPermissionState("missing", "Access to the extension list is off.");
  setStatus("Permission dropped. Your exported files are untouched.", "ok");
}

function exportAs(kind) {
  if (!backup) return;
  if (kind === "json") {
    saveFile(buildJson(backup), backupFileName("json", backup.generatedAt), "application/json");
  } else if (kind === "html") {
    saveFile(buildHtml(backup), backupFileName("html", backup.generatedAt), "text/html");
  } else {
    saveFile(buildMarkdown(backup), backupFileName("md", backup.generatedAt), "text/markdown");
  }
  setStatus("File saved to your downloads folder.", "ok");
}

grantBtn.addEventListener("click", () => void requestPermission());
rescanBtn.addEventListener("click", () => void scan());
revokeBtn.addEventListener("click", () => void revokePermission());
jsonBtn.addEventListener("click", () => exportAs("json"));
htmlBtn.addEventListener("click", () => exportAs("html"));
mdBtn.addEventListener("click", () => exportAs("md"));

async function init() {
  setExportsEnabled(false);
  if (await hasPermission()) {
    setPermissionState("found", "Access granted — reading the list on this machine only.");
    await scan();
    return;
  }
  setPermissionState(
    "missing",
    "This page needs one-time access to the list of installed extensions.",
  );
  grantBtn.hidden = false;
  setStatus("");
}

void init();
