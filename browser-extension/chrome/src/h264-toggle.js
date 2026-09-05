// Liga/desliga o "Forçar H.264" registrando (ou não) o script de página no
// YouTube. Persistido em chrome.storage.local; o registro sobrevive a
// reinícios do navegador (persistAcrossSessions), então só é preciso
// reconciliar quando o estado muda ou o service worker acorda.
const STORAGE_KEY = "omniget_h264_enabled";
const STORAGE_60 = "omniget_h264_block60";
const SCRIPT_ID = "omniget-h264ify";
const MATCHES = ["*://*.youtube.com/*", "*://*.youtube-nocookie.com/*"];

let enabled = false;
let block60 = false;

export async function loadH264State() {
  const r = await chrome.storage.local.get([STORAGE_KEY, STORAGE_60]);
  enabled = r[STORAGE_KEY] === true;
  block60 = r[STORAGE_60] === true;
  await reconcile();
  return { enabled, block60 };
}

export function isH264Enabled() {
  return enabled;
}

export function isBlock60() {
  return block60;
}

async function registered() {
  try {
    const list = await chrome.scripting.getRegisteredContentScripts({ ids: [SCRIPT_ID] });
    return list.length > 0;
  } catch {
    return false;
  }
}

async function reconcile() {
  if (!chrome.scripting?.registerContentScripts) return;
  const has = await registered();
  try {
    if (enabled && !has) {
      await chrome.scripting.registerContentScripts([
        { id: SCRIPT_ID, js: [block60 ? "src/h264ify-60.js" : "src/h264ify.js"], matches: MATCHES, runAt: "document_start", world: "MAIN", persistAcrossSessions: true },
      ]);
    } else if (enabled && has) {
      await chrome.scripting.updateContentScripts([{ id: SCRIPT_ID, js: [block60 ? "src/h264ify-60.js" : "src/h264ify.js"] }]);
    } else if (!enabled && has) {
      await chrome.scripting.unregisterContentScripts({ ids: [SCRIPT_ID] });
    }
  } catch (e) {
    console.warn("[OmniGet] h264ify:", e);
  }
}

export async function setH264(value, block60Value) {
  enabled = Boolean(value);
  if (typeof block60Value === "boolean") block60 = block60Value;
  await chrome.storage.local.set({ [STORAGE_KEY]: enabled, [STORAGE_60]: block60 });
  await reconcile();
  return { enabled, block60 };
}
