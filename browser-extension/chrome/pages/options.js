import {
  loadBridgeConfig,
  saveBridgeConfig,
  checkBridgeHealth,
  discoverBridgeEndpoint,
  trimEndpoint,
  autoPair,
} from "../src/bridge-client.js";
import {
  applyRegexRules,
  compileRegexRules,
  normalizeUserRules,
  parseSizeRule,
} from "../src/capture-rules.js";
import {
  defaultCaptureRules,
  loadCaptureRules,
  resetCaptureRules,
  saveCaptureRules,
} from "../src/capture-store.js";

// ---------------------------------------------------------------------------
// Pure helpers for the capture-rules tables.
//
// They live at the top of this module, outside the DOM wiring below, so
// tests/capture-rules-ui.test.mjs can import them under `node --test` where
// there is no document. The rule logic itself belongs to src/capture-rules.js
// and is only ever called from here, never re-implemented.
// ---------------------------------------------------------------------------

const SIZE_UNITS = Object.freeze([
  Object.freeze({ label: "GB", bytes: 1024 * 1024 * 1024 }),
  Object.freeze({ label: "MB", bytes: 1024 * 1024 }),
  Object.freeze({ label: "KB", bytes: 1024 }),
]);

const SIZE_PHRASES = Object.freeze({
  ">=": size => `at least ${size}`,
  ">": size => `larger than ${size}`,
  "<=": size => `at most ${size}`,
  "<": size => `smaller than ${size}`,
  "=": size => `exactly ${size}`,
  "!=": size => `any size except ${size}`,
});

export const SIZE_RULE_HELP = "Not a size rule — try \">=50 KB\", \"<1 GB\" or \"500-1000 MB\".";

/**
 * Render a byte count in the largest unit it actually reaches, with at most
 * two decimals and no trailing zeros. Base 1024, exactly like parseSizeRule,
 * so "1000 MB" is never rounded up into a "1 GB" the rule does not mean.
 * @param {number} bytes
 * @returns {string}
 */
export function formatRuleBytes(bytes) {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes < 0) return "";
  for (const unit of SIZE_UNITS) {
    if (bytes < unit.bytes) continue;
    return `${trimNumber(bytes / unit.bytes)} ${unit.label}`;
  }
  return `${trimNumber(bytes)} ${bytes === 1 ? "byte" : "bytes"}`;
}

/**
 * @param {number} value
 * @returns {string} the number with at most two decimals and no trailing zeros
 */
function trimNumber(value) {
  return String(Number(value.toFixed(2)));
}

/**
 * Say in plain English what a size rule means, for the live hint next to the
 * field. An empty field is not an error: it means "no size limit".
 * @param {string|number|null|undefined} input
 * @returns {{state: "empty"|"ok"|"invalid", text: string}}
 */
export function describeSizeRule(input) {
  const text = input === null || input === undefined ? "" : String(input).trim();
  if (text === "") return { state: "empty", text: "No size limit" };

  const rule = parseSizeRule(text);
  if (!rule) return { state: "invalid", text: SIZE_RULE_HELP };

  if (rule.operator === "~") {
    return {
      state: "ok",
      text: `between ${formatRuleBytes(rule.min)} and ${formatRuleBytes(rule.max)}`,
    };
  }

  const phrase = SIZE_PHRASES[rule.operator];
  if (!phrase) return { state: "invalid", text: SIZE_RULE_HELP };
  return { state: "ok", text: phrase(formatRuleBytes(rule.size)) };
}

/**
 * Keep a size the user typed only when it parses. A half-typed rule is stored
 * as "no limit" rather than as garbage the sniffer would have to guess about.
 * @param {any} size
 * @returns {string|null}
 */
function keepSize(size) {
  const text = typeof size === "string" ? size.trim() : "";
  if (text === "") return null;
  return parseSizeRule(text) === null ? null : text;
}

/**
 * @param {any} rows
 * @param {"ext"|"type"} key
 * @returns {Array<object>} rows with a non-empty key field
 */
function cleanKeyedRows(rows, key) {
  if (!Array.isArray(rows)) return [];
  const out = [];
  for (const row of rows) {
    if (!row || typeof row !== "object") continue;
    const value = typeof row[key] === "string" ? row[key].trim() : "";
    if (value === "") continue;
    out.push({ [key]: value, size: keepSize(row.size), enabled: row.enabled !== false });
  }
  return out;
}

/**
 * @param {any} rows
 * @returns {Array<object>} regex rows with a non-empty pattern
 */
function cleanRegexRows(rows) {
  if (!Array.isArray(rows)) return [];
  const out = [];
  for (const row of rows) {
    if (!row || typeof row !== "object") continue;
    const pattern = typeof row.pattern === "string" ? row.pattern.trim() : "";
    if (pattern === "") continue;
    const flags = typeof row.flags === "string" ? row.flags.trim() : "";
    out.push({
      pattern,
      flags: flags === "" ? "ig" : flags,
      action: row.action === "block" ? "block" : "accept",
      ext: typeof row.ext === "string" ? row.ext.trim() : "",
      enabled: row.enabled !== false,
    });
  }
  return out;
}

/**
 * Build the canonical rule object out of the three tables as the user left
 * them. Blank rows are dropped and every key goes through normalizeUserRules,
 * so a typed "MP4" is stored as ".mp4" and "Video/MP4" as "video/mp4".
 * @param {{extensions?: Array, contentTypes?: Array, regex?: Array}} draft
 * @returns {{extensions: Array<object>, contentTypes: Array<object>, regex: Array<object>}}
 */
export function buildCaptureRules(draft) {
  const source = draft && typeof draft === "object" ? draft : {};
  return normalizeUserRules({
    extensions: cleanKeyedRows(source.extensions, "ext"),
    contentTypes: cleanKeyedRows(source.contentTypes, "type"),
    regex: cleanRegexRows(source.regex),
  });
}

/**
 * Normalise a single extension or content type the way the store would, so
 * the field can be tidied when the user leaves it. Text that cannot be
 * normalised is handed back untouched: the row is flagged, not silently eaten.
 * @param {"ext"|"type"} kind
 * @param {string} value
 * @returns {string}
 */
export function normalizeRuleKey(kind, value) {
  const text = typeof value === "string" ? value.trim() : "";
  if (text === "") return "";
  const bucket = kind === "ext" ? "extensions" : "contentTypes";
  const field = kind === "ext" ? "ext" : "type";
  const normalized = normalizeUserRules({ [bucket]: [{ [field]: text }] })[bucket];
  return normalized.length > 0 ? normalized[0][field] : text;
}

/**
 * Describe what applyRegexRules would do with a URL, for the test field under
 * the regex table. Rules are numbered by their row, so the answer points at
 * the line the user has to fix.
 * @param {string} url
 * @param {Array<object>} rules regex rows as edited in the UI
 * @returns {{state: "empty"|"blocked"|"accepted"|"no-match", text: string, url: string}}
 */
export function describeRegexTest(url, rules) {
  const text = typeof url === "string" ? url.trim() : "";
  if (text === "") return { state: "empty", text: "", url: "" };

  const compiled = compileRegexRules(Array.isArray(rules) ? rules : []);
  const result = applyRegexRules(text, compiled);
  if (!result) {
    return {
      state: "no-match",
      text: "No rule matched — the extension and content-type tables decide this URL.",
      url: "",
    };
  }

  const position = compiled.indexOf(result.rule) + 1;
  const label = position > 0 ? `Rule ${position}` : "A rule";
  if (result.action === "block") {
    return { state: "blocked", text: `${label} blocks this URL — it is never captured.`, url: "" };
  }

  const ext = typeof result.ext === "string" ? result.ext.replace(/^\.+/, "") : "";
  const savedAs = ext === "" ? "" : ` (saved as .${ext})`;
  if (result.url === text) {
    return { state: "accepted", text: `${label} matches — captured as is${savedAs}.`, url: result.url };
  }
  return { state: "accepted", text: `${label} matches — captured as ${result.url}${savedAs}`, url: result.url };
}

// ---------------------------------------------------------------------------
// Page wiring. Everything below touches the DOM, so it only runs in the
// browser; under `node --test` this module is imported for the helpers above.
// ---------------------------------------------------------------------------

if (typeof document !== "undefined") {
  const endpointInput = document.getElementById("endpoint");
  const tokenInput = document.getElementById("token");
  const revealBtn = document.getElementById("reveal");
  const form = document.getElementById("pair-form");
  const testBtn = document.getElementById("test");
  const pairNowBtn = document.getElementById("pair-now");
  const statusEl = document.getElementById("status");
  const welcomeHeader = document.getElementById("welcome-header");
  const settingsHeader = document.getElementById("settings-header");
  const discoveryEl = document.getElementById("discovery-status");
  const discoveryMessage = document.getElementById("discovery-message");
  const endpointHint = document.getElementById("endpoint-hint");
  const advancedDetails = document.getElementById("advanced");

  const FALLBACK_ENDPOINT = "http://127.0.0.1:47720";

  function resolvedEndpoint() {
    return trimEndpoint(endpointInput.value) || FALLBACK_ENDPOINT;
  }

  function setStatus(message, kind) {
    statusEl.textContent = message ?? "";
    statusEl.classList.remove("ok", "error");
    if (kind === "ok") statusEl.classList.add("ok");
    if (kind === "error") statusEl.classList.add("error");
  }

  function setDiscovery(state, message) {
    discoveryEl.classList.remove("found", "missing");
    if (state === "found") discoveryEl.classList.add("found");
    if (state === "missing") discoveryEl.classList.add("missing");
    discoveryMessage.textContent = message;
  }

  // --- Auto-pairing -----------------------------------------------------------
  // The desktop app opens a single-use ~120s pairing window when the user
  // clicks "Pair extension" in Settings. While this page is visible we poll
  // `GET /v1/pair` every 5 seconds so pairing completes within seconds of that
  // click (the background service worker only retries once per minute).
  const AUTOPAIR_POLL_MS = 5000;
  let pairPollTimer = null;

  async function onPairedSuccess() {
    const { endpoint, token } = await loadBridgeConfig();
    endpointInput.value = endpoint || "";
    tokenInput.value = token || "";
    setDiscovery("found", `Paired with OmniGet at ${endpoint}.`);
    setStatus("Paired automatically — you're all set.", "ok");
    stopPairPolling();
  }

  async function tryAutoPair() {
    const result = await autoPair().catch(() => ({ ok: false }));
    if (!result?.ok) return false;
    if (result.reason === "already-paired") {
      stopPairPolling();
      return true;
    }
    await onPairedSuccess();
    return true;
  }

  function startPairPolling() {
    if (pairPollTimer !== null) return;
    pairPollTimer = setInterval(() => {
      void tryAutoPair();
    }, AUTOPAIR_POLL_MS);
  }

  function stopPairPolling() {
    if (pairPollTimer !== null) {
      clearInterval(pairPollTimer);
      pairPollTimer = null;
    }
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      stopPairPolling();
      return;
    }
    loadBridgeConfig().then(({ token }) => {
      if (!token) {
        void tryAutoPair();
        startPairPolling();
      }
    });
  });

  async function init() {
    const { endpoint, token } = await loadBridgeConfig();
    const alreadyPaired = Boolean(token);

    // While unpaired, keep trying to grab the token automatically: once
    // immediately (the user may already have a pairing window open in the
    // app) and then every few seconds while this page stays visible.
    if (!alreadyPaired) {
      void tryAutoPair();
      if (!document.hidden) startPairPolling();
    }

    // Show the welcome heading on first run, the regular settings heading
    // once the user is already paired (they're here to inspect / change).
    if (alreadyPaired) {
      settingsHeader.hidden = false;
    } else {
      welcomeHeader.hidden = false;
    }

    endpointInput.value = endpoint || "";
    tokenInput.value = token || "";

    // Skip auto-discovery if the user is already paired AND we have a stored
    // endpoint that responds — they're probably here to change the token, no
    // need to overwrite the URL they trust.
    if (alreadyPaired) {
      const result = await checkBridgeHealth(endpoint);
      if (result.ok) {
        const versionSuffix = result.version ? ` (v${result.version})` : "";
        setDiscovery("found", `Connected to OmniGet${versionSuffix} at ${endpoint}.`);
        return;
      }
      setDiscovery(
        "missing",
        `Couldn't reach the saved endpoint ${endpoint}. Probing default ports…`
      );
    }

    const found = await discoverBridgeEndpoint();
    if (found) {
      endpointInput.value = found.endpoint;
      const versionSuffix = found.version ? ` (v${found.version})` : "";
      setDiscovery(
        "found",
        `Found OmniGet${versionSuffix} on ${found.endpoint}. Paste the token from OmniGet → Settings → Network → Browser extension to finish.`
      );
      endpointHint.textContent =
        "Auto-detected — change only if your OmniGet runs on a different host.";
      return;
    }

    // Discovery failed: open the Advanced disclosure so the user can supply
    // the endpoint manually.
    if (advancedDetails) advancedDetails.open = true;
    setDiscovery(
      "missing",
      "OmniGet doesn't seem to be running. Launch the desktop app, then refresh this page — or set the endpoint manually below."
    );
  }

  revealBtn.addEventListener("click", () => {
    const next = tokenInput.type === "password" ? "text" : "password";
    tokenInput.type = next;
    revealBtn.textContent = next === "password" ? "Show" : "Hide";
    revealBtn.setAttribute("aria-pressed", String(next !== "password"));
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const endpoint = resolvedEndpoint();
    const token = tokenInput.value.trim();
    if (!token) {
      setStatus("Paste the pairing token first.", "error");
      return;
    }
    await saveBridgeConfig({ endpoint, token });
    setStatus("Saved. The extension will use this token from now on.", "ok");
  });

  if (pairNowBtn) {
    pairNowBtn.addEventListener("click", async () => {
      setStatus("Trying to pair automatically…");
      const paired = await tryAutoPair();
      if (paired) return;
      setStatus(
        "No open pairing window found. In OmniGet, go to Settings → Network → Browser extension and click \"Pair extension\", then try again (or just wait — this page keeps retrying).",
        "error"
      );
      if (!document.hidden) startPairPolling();
    });
  }

  testBtn.addEventListener("click", async () => {
    const endpoint = resolvedEndpoint();
    setStatus("Testing connection…");
    const result = await checkBridgeHealth(endpoint);
    if (result.ok) {
      const versionSuffix = result.version ? ` (v${result.version})` : "";
      setStatus(`Connected to OmniGet${versionSuffix} at ${endpoint}.`, "ok");
    } else {
      setStatus(
        `Could not reach OmniGet at ${endpoint}. Make sure the app is running.`,
        "error"
      );
    }
  });

  init();

  // --- Capture rules --------------------------------------------------------
  // Three editable tables (extensions, content types, regex) over the same
  // rule objects the sniffer reads. Edits save themselves after a short pause
  // instead of behind a Save button: every field here is independent, and a
  // half-finished table left unsaved is worse than one saved a beat late. A
  // row whose size rule does not parse holds the save back until it is fixed.
  const captureDetails = document.getElementById("capture-rules");
  const extBody = document.getElementById("ext-rows");
  const typeBody = document.getElementById("type-rows");
  const regexBody = document.getElementById("regex-rows");
  const rulesStatus = document.getElementById("rules-status");
  const regexTestInput = document.getElementById("regex-test");
  const regexTestResult = document.getElementById("regex-test-result");
  const resetRulesBtn = document.getElementById("rules-reset");

  const RULES_SAVE_DEBOUNCE_MS = 500;
  const RESET_ARM_MS = 5000;

  let rulesLoaded = false;
  let rulesSaveTimer = null;
  let resetArmedTimer = null;

  function setRulesStatus(message, kind) {
    if (!rulesStatus) return;
    rulesStatus.textContent = message ?? "";
    rulesStatus.classList.remove("ok", "error");
    if (kind === "ok") rulesStatus.classList.add("ok");
    if (kind === "error") rulesStatus.classList.add("error");
  }

  function ruleInput(value, placeholder, extraClass) {
    const input = document.createElement("input");
    input.type = "text";
    input.value = typeof value === "string" ? value : "";
    input.placeholder = placeholder;
    input.autocomplete = "off";
    input.spellcheck = false;
    input.className = extraClass ? `rule-input ${extraClass}` : "rule-input";
    return input;
  }

  function ruleCheckbox(checked, label) {
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = checked;
    input.className = "rule-toggle";
    input.setAttribute("aria-label", label);
    input.addEventListener("change", scheduleRulesSave);
    return input;
  }

  function removeRowButton(tr, after) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "button small ghost";
    button.textContent = "Remove";
    button.addEventListener("click", () => {
      tr.remove();
      if (after) after();
      scheduleRulesSave();
    });
    return button;
  }

  function addCell(tr, child, className) {
    const td = document.createElement("td");
    if (className) td.className = className;
    if (child) td.append(child);
    tr.append(td);
    return td;
  }

  // A size cell carries its own live translation: ">=50 KB" reads back as
  // "at least 50 KB", and anything parseSizeRule rejects marks the row.
  function sizeCell(tr) {
    const wrap = document.createElement("div");
    wrap.className = "rule-stack";
    const input = ruleInput("", "no limit", "size");
    const note = document.createElement("span");
    note.className = "hint";
    wrap.append(input, note);

    function refresh() {
      const described = describeSizeRule(input.value);
      note.textContent = described.text;
      note.classList.toggle("error", described.state === "invalid");
      input.classList.toggle("invalid", described.state === "invalid");
      tr.dataset.badSize = described.state === "invalid" ? "1" : "";
    }

    input.addEventListener("input", () => {
      refresh();
      scheduleRulesSave();
    });
    return { wrap, input, refresh };
  }

  // Extension and content-type rows differ only in their key field, so they
  // share one builder.
  function keyedRow(kind, rule) {
    const tr = document.createElement("tr");
    const field = kind === "ext" ? "ext" : "type";
    const enabled = ruleCheckbox(rule?.enabled !== false, kind === "ext" ? "Use this extension" : "Use this content type");
    const key = ruleInput(rule?.[field] ?? "", kind === "ext" ? ".mp4" : "video/mp4");
    const size = sizeCell(tr);

    function refreshKey() {
      const text = key.value.trim();
      const bad = text !== "" && !isNormalizedKey(kind, text);
      key.classList.toggle("invalid", bad);
      tr.dataset.badKey = bad ? "1" : "";
    }

    key.addEventListener("input", () => {
      refreshKey();
      scheduleRulesSave();
    });
    key.addEventListener("blur", () => {
      const normalized = normalizeRuleKey(kind, key.value);
      if (normalized !== key.value) {
        key.value = normalized;
        scheduleRulesSave();
      }
      refreshKey();
    });

    addCell(tr, enabled, "col-on");
    addCell(tr, key);
    addCell(tr, size.wrap);
    addCell(tr, removeRowButton(tr), "col-remove");

    size.input.value = typeof rule?.size === "string" ? rule.size : "";
    size.refresh();
    refreshKey();

    tr.readRule = () => ({ [field]: key.value, size: size.input.value, enabled: enabled.checked });
    return tr;
  }

  /**
   * True when the text is already in the shape the store keeps (".mp4",
   * "video/mp4"), which is how an unusable key ("foo bar") is told apart from
   * one that simply needs no tidying.
   */
  function isNormalizedKey(kind, text) {
    const bucket = kind === "ext" ? "extensions" : "contentTypes";
    return normalizeUserRules({ [bucket]: [kind === "ext" ? { ext: text } : { type: text }] })[bucket].length > 0;
  }

  function regexRow(rule) {
    const tr = document.createElement("tr");
    const enabled = ruleCheckbox(rule?.enabled !== false, "Use this rule");
    const pattern = ruleInput(rule?.pattern ?? "", "(^https://cdn\\.example\\.com/.*)&bytestart=.*", "pattern");
    const error = document.createElement("div");
    error.className = "hint error";
    const patternWrap = document.createElement("div");
    patternWrap.className = "rule-stack";
    patternWrap.append(pattern, error);

    const flags = ruleInput(rule?.flags ?? "ig", "ig", "flags");
    const ext = ruleInput(rule?.ext ?? "", "auto", "ext");

    const action = document.createElement("select");
    action.className = "rule-input action";
    action.setAttribute("aria-label", "What this rule does");
    for (const value of ["accept", "block"]) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = value === "accept" ? "Accept" : "Block";
      action.append(option);
    }
    action.value = rule?.action === "block" ? "block" : "accept";

    // Compile on every keystroke: an unfinished pattern must never throw its
    // way out of here, and the message from the engine is the useful one.
    function refresh() {
      const text = pattern.value.trim();
      const [compiled] = compileRegexRules([{ pattern: text, flags: flags.value, enabled: true }]);
      const message = text === "" || !compiled ? "" : compiled.error ?? "";
      error.textContent = message;
      pattern.classList.toggle("invalid", message !== "");
      tr.dataset.badPattern = message !== "" ? "1" : "";
    }

    for (const field of [pattern, flags, ext]) {
      field.addEventListener("input", () => {
        refresh();
        refreshRegexTest();
        scheduleRulesSave();
      });
    }
    action.addEventListener("change", () => {
      refreshRegexTest();
      scheduleRulesSave();
    });
    enabled.addEventListener("change", refreshRegexTest);

    addCell(tr, enabled, "col-on");
    addCell(tr, patternWrap);
    addCell(tr, flags, "col-flags");
    addCell(tr, action, "col-action");
    addCell(tr, ext, "col-ext");
    addCell(tr, removeRowButton(tr, refreshRegexTest), "col-remove");

    refresh();
    tr.readRule = () => ({
      pattern: pattern.value,
      flags: flags.value,
      action: action.value,
      ext: ext.value,
      enabled: enabled.checked,
    });
    return tr;
  }

  function readRows(body) {
    if (!body) return [];
    return Array.from(body.rows)
      .filter(tr => typeof tr.readRule === "function")
      .map(tr => tr.readRule());
  }

  function currentDraft() {
    return {
      extensions: readRows(extBody),
      contentTypes: readRows(typeBody),
      regex: readRows(regexBody),
    };
  }

  function firstBadRow() {
    for (const body of [extBody, typeBody, regexBody]) {
      if (!body) continue;
      for (const tr of body.rows) {
        if (tr.dataset.badSize === "1") return "size rule";
        if (tr.dataset.badKey === "1") return "extension or content type";
        if (tr.dataset.badPattern === "1") return "pattern";
      }
    }
    return null;
  }

  function refreshRegexTest() {
    if (!regexTestInput || !regexTestResult) return;
    const result = describeRegexTest(regexTestInput.value, readRows(regexBody));
    regexTestResult.textContent = result.text;
    regexTestResult.classList.toggle("ok", result.state === "accepted");
    regexTestResult.classList.toggle("error", result.state === "blocked");
  }

  function renderRules(rules) {
    if (!extBody || !typeBody || !regexBody) return;
    extBody.replaceChildren(...rules.extensions.map(rule => keyedRow("ext", rule)));
    typeBody.replaceChildren(...rules.contentTypes.map(rule => keyedRow("type", rule)));
    regexBody.replaceChildren(...rules.regex.map(rule => regexRow(rule)));
    refreshRegexTest();
  }

  function scheduleRulesSave() {
    disarmReset();
    if (rulesSaveTimer !== null) clearTimeout(rulesSaveTimer);
    rulesSaveTimer = setTimeout(() => {
      rulesSaveTimer = null;
      void saveRulesNow();
    }, RULES_SAVE_DEBOUNCE_MS);
  }

  async function saveRulesNow() {
    const bad = firstBadRow();
    if (bad) {
      setRulesStatus(`Fix the highlighted ${bad} — nothing was saved.`, "error");
      return;
    }
    try {
      await saveCaptureRules(buildCaptureRules(currentDraft()));
      setRulesStatus("Capture rules saved.", "ok");
    } catch {
      setRulesStatus("Couldn't save the capture rules.", "error");
    }
  }

  function addRow(body, node) {
    if (!body) return;
    body.append(node);
    const input = node.querySelector("input[type=\"text\"]");
    if (input) input.focus();
  }

  // "Restore defaults" confirms in place: the button asks, the second click
  // inside a few seconds does it. No confirm() dialog on an options page.
  function armReset() {
    if (!resetRulesBtn) return;
    resetRulesBtn.textContent = "Are you sure?";
    resetRulesBtn.classList.add("danger");
    resetArmedTimer = setTimeout(disarmReset, RESET_ARM_MS);
  }

  function disarmReset() {
    if (resetArmedTimer !== null) {
      clearTimeout(resetArmedTimer);
      resetArmedTimer = null;
    }
    if (!resetRulesBtn) return;
    resetRulesBtn.textContent = "Restore defaults";
    resetRulesBtn.classList.remove("danger");
  }

  if (resetRulesBtn) {
    resetRulesBtn.addEventListener("click", async () => {
      if (resetArmedTimer === null) {
        armReset();
        setRulesStatus("Click again to drop every change and restore the defaults.");
        return;
      }
      disarmReset();
      if (rulesSaveTimer !== null) {
        clearTimeout(rulesSaveTimer);
        rulesSaveTimer = null;
      }
      try {
        renderRules(await resetCaptureRules());
        setRulesStatus("Capture rules restored to the defaults.", "ok");
      } catch {
        setRulesStatus("Couldn't restore the defaults.", "error");
      }
    });
    resetRulesBtn.addEventListener("blur", disarmReset);
  }

  document.getElementById("ext-add")?.addEventListener("click", () => {
    addRow(extBody, keyedRow("ext", { ext: "", size: "", enabled: true }));
  });
  document.getElementById("type-add")?.addEventListener("click", () => {
    addRow(typeBody, keyedRow("type", { type: "", size: "", enabled: true }));
  });
  document.getElementById("regex-add")?.addEventListener("click", () => {
    addRow(regexBody, regexRow({ pattern: "", flags: "ig", action: "accept", ext: "", enabled: true }));
  });

  regexTestInput?.addEventListener("input", refreshRegexTest);

  // The tables are advanced settings: build them the first time the section is
  // opened, not on every visit to the pairing page.
  if (captureDetails) {
    captureDetails.addEventListener("toggle", async () => {
      if (!captureDetails.open || rulesLoaded) return;
      rulesLoaded = true;
      try {
        renderRules(await loadCaptureRules());
      } catch {
        renderRules(defaultCaptureRules());
      }
    });
  }
}
