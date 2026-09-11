// Loads the user's capture tables from storage and keeps a compiled copy in
// memory. Mirrors the shape of blocked-hosts.js: read once at module load,
// recompile on storage.onChanged, never make the request path touch storage.

import {
  DEFAULT_MEDIA_CONTENT_TYPES,
  DEFAULT_MEDIA_EXTENSIONS,
  DEFAULT_REGEX_RULES,
  compileRegexRules,
  normalizeUserRules,
} from "./capture-rules.js";

export const CAPTURE_RULES_KEY = "captureRules";

export function defaultCaptureRules() {
  return {
    extensions: DEFAULT_MEDIA_EXTENSIONS.map(rule => ({ ...rule })),
    contentTypes: DEFAULT_MEDIA_CONTENT_TYPES.map(rule => ({ ...rule })),
    regex: DEFAULT_REGEX_RULES.map(rule => ({ ...rule })),
  };
}

// A saved table replaces the defaults rather than merging with them: the
// options page hands the user the full default table to edit, so a stored table
// is already the complete answer. An empty bucket means "the user cleared this
// table", which has to survive a reload.
export function resolveCaptureRules(stored) {
  const defaults = defaultCaptureRules();
  if (!stored || typeof stored !== "object") return defaults;
  const normalized = normalizeUserRules(stored);
  const pick = (key) => (Array.isArray(stored[key]) ? normalized[key] : defaults[key]);
  return {
    extensions: pick("extensions"),
    contentTypes: pick("contentTypes"),
    regex: pick("regex"),
  };
}

let activeRules = defaultCaptureRules();
let compiledRegex = compileRegexRules(activeRules.regex);

export function getCaptureRules() {
  return activeRules;
}

export function getCompiledRegexRules() {
  return compiledRegex;
}

function applyRules(rules) {
  activeRules = rules;
  compiledRegex = compileRegexRules(rules.regex);
  return activeRules;
}

export async function loadCaptureRules() {
  try {
    const data = await chrome.storage.local.get(CAPTURE_RULES_KEY);
    return applyRules(resolveCaptureRules(data?.[CAPTURE_RULES_KEY]));
  } catch {
    return applyRules(defaultCaptureRules());
  }
}

export async function saveCaptureRules(rules) {
  const resolved = resolveCaptureRules(rules);
  await chrome.storage.local.set({ [CAPTURE_RULES_KEY]: resolved });
  return applyRules(resolved);
}

export async function resetCaptureRules() {
  await chrome.storage.local.remove(CAPTURE_RULES_KEY).catch(() => {});
  return applyRules(defaultCaptureRules());
}

if (typeof chrome !== "undefined" && chrome?.storage?.local) {
  loadCaptureRules();
  if (chrome.storage.onChanged && typeof chrome.storage.onChanged.addListener === "function") {
    chrome.storage.onChanged.addListener((changes, areaName) => {
      if (areaName !== "local") return;
      if (!(CAPTURE_RULES_KEY in changes)) return;
      loadCaptureRules();
    });
  }
}
