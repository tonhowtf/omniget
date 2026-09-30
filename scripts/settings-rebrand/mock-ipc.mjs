/**
 * Browser-only fixture. Never imports Tauri, opens the app, or calls a backend.
 * Serialized into the page by Playwright (addInitScript), so it must stay self-contained.
 *
 * Options:
 *   defaults       synthetic AppSettings document (docs/settings-rebrand/defaults-fixture.json)
 *   language/theme case locale and colour mode
 *   family/state   legacy per-family state injection (loading/error/empty/disabled/expanded)
 *   settingsState  v3 envelope scenario: ready | loading | get-failed | patch-rejected |
 *                  pending-effect | recovered
 *   delays         { command: ms } late responses (used by the navigation/focus cases)
 */
export function installSyntheticIPC({ defaults, language, theme, family = "", state = "baseline", settingsState = "ready", delays = {} }) {
  const copy = value => (value === undefined ? undefined : structuredClone(value));
  const key = "settings-rebrand.mock.settings.v3";
  const initial = copy(defaults);
  initial.appearance.language = language;
  // v3 appearance: explicit mode/preset, import already committed (migration_version=1).
  Object.assign(initial.appearance, { mode: theme, preset: "omniget-desktop", legacy_theme: null, migration_version: 1, theme });
  initial.schema_version = 3;
  // Documented harness-only overrides: bypass onboarding/legal overlays, not IPC security.
  initial.onboarding_completed = true;
  initial.legal_acknowledged = true;
  if (family === "network" && state !== "disabled") initial.proxy.enabled = true;
  if (state === "expanded") Object.assign(initial.download, {
    download_subtitles: true, include_auto_subtitles: true, hotkey_enabled: true,
    music_hotkey_enabled: true, youtube_sponsorblock: true, bilibili_danmaku_enabled: true,
    bilibili_cover_sidecar: true,
  });
  // Private document (never leaves the mock unredacted). Secret values are fictitious
  // sentinels so a leak into the DOM is detectable; has_* markers are derived from them.
  let stored = null;
  try { stored = JSON.parse(localStorage.getItem(key) || "null"); } catch { stored = null; }
  let settings = stored?.settings || initial;
  let revision = BigInt(stored?.revision || "7");
  if (!stored) {
    settings.proxy.password = family === "network" ? "SYNTHETIC-SECRET-PROXY" : "";
    settings.advanced.twitter_manual_cookie = "";
    settings.bridge = { ...(settings.bridge || {}), token: "SYNTHETIC-SECRET-BRIDGE" };
  }
  let pendingEffects = settingsState === "pending-effect"
    ? [{ id: "network.proxy", targetRevision: String(revision), state: "failed", code: "apply_failed", retryable: true }]
    : [];
  const loadStatus = settingsState === "recovered"
    ? { kind: "recovered", fields: ["download.max_concurrent"], recoveryId: "00000000-0000-4000-8000-000000000001" }
    : { kind: "loaded" };

  const initialAI = { provider: "local", kind: "", model: "fixture-model", local_base_url: "http://localhost:11434/v1", has_openai_key: false, has_anthropic_key: false };
  let ai = JSON.parse(localStorage.getItem(key + ".ai") || "null") || initialAI;
  if (family === "ai" && ["empty", "disabled"].includes(state)) ai = { ...ai, provider: "none", model: "" };
  const profile = { public_key_b64: "SYNTHETIC_PUBLIC_KEY", fingerprint: "1234567890abcdef1234567890abcdef", nickname: "Pessoa de teste", skin: { id: "omni-default", tint: [110, 139, 255] }, created_at_ms: 1700000000000 };
  let channels = state === "empty" ? [] : [{ id: "fixture-channel", url: "https://example.invalid/channel", title: "Canal sintético de teste", added_at_ms: 1700000000000, last_checked_ms: null, enabled: true, auto_download: false, interval_minutes: 60 }];
  let rules = [];
  const calls = [], unexpected = [], listeners = {}, callbacks = {};
  const behavior = { reject: [], hold: [], delay: { ...delays }, dialog: null, rejectWith: {} };
  const familyCommands = {
    ai: ["ai_get_config"], profile: ["profile_get"],
    network: ["get_bridge_info"], channels: ["channels_list"],
    dependencies: ["check_dependencies", "browser_extension_status"],
    advanced: ["content_store_stats", "get_portable_info"], rules: ["list_rules"],
    world: ["pet_capabilities", "world_exists"], cookies: ["cookies_list", "cookies_health"],
    indicators: ["usage_tray_view"],
  };
  if (state === "error") behavior.reject = familyCommands[family] || [];
  if (state === "loading") behavior.hold = familyCommands[family] || [];
  if (settingsState === "loading") behavior.hold.push("settings_get_v3", "get_settings");
  if (settingsState === "patch-rejected") behavior.rejectWith.settings_patch_v3 = { kind: "persistence", code: "write_failed", committed: false, fieldErrors: [] };

  // Commands that change product state. A navigation must call none of them.
  // Explicit list first; unknown commands matching the verb pattern also count.
  const MUTATING = [
    "settings_patch_v3", "settings_reset_v3", "settings_import_appearance_v3", "settings_retry_effect_v3",
    "update_settings", "reset_settings", "mark_onboarding_complete", "mark_legal_acknowledged",
    "ai_set_config", "ai_test", "ai_summarize_url", "ai_history_clear",
    "save_rules", "channel_add", "channel_remove", "channel_update", "channel_check_now",
    "profile_set_nickname", "profile_set_skin", "cookies_import", "cookies_delete", "cookies_test",
    "usage_tray_set", "usage_tray_toggle_account", "usage_tray_refresh",
    "league_auto_accept_set", "update_max_concurrent", "rotate_bridge_token", "bridge_open_pairing",
    "browser_extension_export", "dependency_install", "dependency_set_custom_path",
    "download_from_url", "start_download", "cancel_download",
    "pet_open", "pet_close", "pet_set_corner", "pet_set_click_through", "world_close", "world_delete",
    "tool_mcp_set_enabled", "tool_mcp_selftest", "tool_mcp_client_create", "tool_mcp_client_revoke",
    "tool_mcp_execution_grant", "tool_mcp_execution_grant_revoke", "tool_mcp_gateway_grant",
    "tool_mcp_network_grant", "tool_mcp_root_grant", "tool_mcp_root_revoke", "tool_mcp_media_configure",
    "assist_mcp_derived_bot_revoke", "music_import_paths", "music_playlist_add", "music_set_liked",
    "assist_learning_add_case", "assist_learning_delete_case", "assist_learning_evaluate", "assist_learning_forget",
    "assist_learning_observe", "assist_learning_promote", "assist_learning_propose", "assist_learning_revoke",
    "assist_learning_rollback", "assist_learning_settings_save", "assist_learning_export",
  ];
  const MUTATING_PATTERN = /(^|_)(set|save|add|remove|delete|clear|create|update|reset|import|retry|install|rotate|revoke|grant|toggle|refresh|start|stop|send|download|export|promote|rollback|forget|apply|configure|enable|disable|write|rename|move|test|selftest|summarize|evaluate|propose|observe|pair)(_|$)/;
  // Session/UI plumbing that runs on every page load; not product-state mutation.
  const PLUMBING = new Set([
    "native_agent_register", "native_agent_reply", "sync_tray_strings", "usage_tray_sync_strings", "sync_llm_prompts",
    "sync_llm_notify_strings", "llm_ui_focus", "rpc_set_idle_stats", "sync_channels_tray", "register_frontend",
    "register_external_frontend", "start_health_monitor", "stop_health_monitor",
    "plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:event|emit_to",
    "plugin:clipboard-manager|write_text",
  ]);
  // Reads named like get_/list_/…_status never count, unless listed explicitly above.
  const READ_PATTERN = /^(get|list|check|read|fetch|load|is|has|query|preview|validate|probe|describe)_|_(list|status|view|get|stats|info|overview|history|capabilities|exists|health|snippets|budget|peaks|playlists|clients|tracks|prefs|state|dir|path|versions)$/;
  const isMutating = command => {
    if (PLUMBING.has(command)) return false;
    if (MUTATING.includes(command)) return true;
    const bare = command.replace(/^plugin:[^|]+\|/, "");
    return !READ_PATTERN.test(bare) && MUTATING_PATTERN.test(bare);
  };

  let callbackId = 5000;
  const persist = () => { try { localStorage.setItem(key, JSON.stringify({ settings, revision: String(revision) })); } catch { /* storage optional */ } };
  const merge = (base, patch) => {
    for (const [name, value] of Object.entries(patch)) {
      if (value && typeof value === "object" && !Array.isArray(value) && base[name] && typeof base[name] === "object") merge(base[name], value);
      else base[name] = value;
    }
    return base;
  };
  // SettingsPublicV3: explicit redaction, has_* are booleans only.
  const redact = () => {
    const dto = copy(settings);
    dto.proxy.has_password = !!settings.proxy.password; delete dto.proxy.password;
    dto.advanced.has_twitter_manual_cookie = !!settings.advanced.twitter_manual_cookie; delete dto.advanced.twitter_manual_cookie;
    if (dto.bridge) { dto.bridge.has_token = !!settings.bridge?.token; delete dto.bridge.token; }
    dto.schema_version = 3;
    return dto;
  };
  const snapshot = () => ({
    settings: redact(), revision: String(revision), loadStatus: copy(loadStatus),
    applied: pendingEffects.length === 0, pendingEffects: copy(pendingEffects), fieldErrors: [],
  });
  const failedSnapshot = () => ({ settings: null, revision: null, loadStatus: { kind: "failed", code: "read_failed", recoveryId: null }, applied: false, pendingEffects: [], fieldErrors: [] });
  const apiError = (kind, code, extra = {}) => ({ kind, code, fieldErrors: [], ...extra });
  const SECRET_PATHS = [["proxy", "password"], ["advanced", "twitter_manual_cookie"], ["bridge", "token"]];
  const PROTECTED_TOP = new Set(["schema_version", "_settings_revision"]);
  const emitChanged = () => setTimeout(() => window.__SETTINGS_AUDIT__?.emit("settings-v3-changed", { revision: String(revision) }), 0);
  const commit = mutate => {
    const before = JSON.stringify(settings);
    mutate();
    if (JSON.stringify(settings) !== before) {
      revision += 1n;
      if (pendingEffects.length) pendingEffects = pendingEffects.map(effect => ({ ...effect, targetRevision: String(revision) }));
      persist(); emitChanged();
    }
    return snapshot();
  };
  const checkRevision = request => {
    if (settingsState === "get-failed") throw apiError("unavailable", "load_failed");
    if (typeof request?.expectedRevision !== "string" || !/^(0|[1-9]\d*)$/.test(request.expectedRevision)) throw apiError("validation", "invalid_request", { fieldErrors: [{ path: "expectedRevision", code: "invalid_type" }] });
    if (BigInt(request.expectedRevision) !== revision) throw apiError("conflict", "revision_conflict", { currentRevision: String(revision) });
  };
  const applySecret = (path, operation) => {
    if (!operation || operation.op === "keep") return;
    const [group, field] = path;
    if (operation.op === "set" && typeof operation.value === "string" && operation.value) settings[group][field] = operation.value;
    else if (operation.op === "remove") settings[group][field] = "";
    else throw apiError("validation", "invalid_request", { fieldErrors: [{ path: path.join("."), code: "invalid_value" }] });
  };

  const nop = new Set([
    "native_agent_register", "native_agent_reply", "sync_tray_strings", "usage_tray_sync_strings", "sync_llm_prompts", "sync_llm_notify_strings", "llm_ui_focus",
    "rpc_set_idle_stats", "rpc_test", "sync_channels_tray", "mark_onboarding_complete", "mark_legal_acknowledged", "start_health_monitor", "stop_health_monitor",
    "register_frontend", "world_close", "update_max_concurrent", "league_auto_accept_set", "pet_open", "pet_close", "pet_set_corner", "pet_set_click_through",
  ]);
  const usageTray = { prefs: { enabled: false, accounts: [], seeded: true, shows: "session", percent_text: true, colored: true, warn: 80, critical: 95 }, catalog: [] };
  const respond = (command, args) => {
    if (nop.has(command)) return null;
    switch (command) {
      case "plugin:event|listen": { const id = ++callbackId; (listeners[args.event] ||= []).push({ id, callback: args.handler }); return id; }
      case "plugin:event|unlisten": return null;
      case "plugin:event|emit": case "plugin:event|emit_to": return null;
      case "plugin:app|version": return "0.10.1-harness";
      case "plugin:app|name": return "OmniGet — frontend harness";
      case "plugin:app|tauri_version": return "2.0.0";
      case "plugin:updater|check": return null;
      case "plugin:window|is_maximized": case "plugin:window|is_fullscreen": return false;
      case "plugin:window|is_focused": return true;
      case "plugin:window|scale_factor": return 1;
      case "plugin:window|inner_size": return { width: innerWidth, height: innerHeight };
      case "plugin:window|available_monitors": return [];
      case "plugin:dialog|open": case "plugin:dialog|save": return behavior.dialog;
      case "plugin:clipboard-manager|read_text": return "";
      case "plugin:clipboard-manager|write_text": return null;
      // ---- v3 contract (06-api-contract.md)
      case "settings_get_v3": return settingsState === "get-failed" ? failedSnapshot() : snapshot();
      case "settings_patch_v3": {
        const request = args.request || {};
        checkRevision(request);
        const patch = request.patch;
        if (!patch || typeof patch !== "object" || Array.isArray(patch)) throw apiError("validation", "invalid_request", { fieldErrors: [{ path: "patch", code: "invalid_type" }] });
        const fieldErrors = [];
        for (const [group, field] of SECRET_PATHS) if (patch[group] && typeof patch[group] === "object" && field in patch[group]) fieldErrors.push({ path: `${group}.${field}`, code: "protected_field" });
        for (const name of Object.keys(patch)) if (PROTECTED_TOP.has(name) || !(name in settings)) fieldErrors.push({ path: name, code: PROTECTED_TOP.has(name) ? "protected_field" : "unknown_field" });
        for (const marker of ["has_password", "has_twitter_manual_cookie", "has_token", "migration_version", "legacy_theme"]) for (const group of Object.values(patch)) if (group && typeof group === "object" && marker in group) fieldErrors.push({ path: marker, code: "protected_field" });
        if (fieldErrors.length) throw apiError("validation", "invalid_request", { fieldErrors });
        return commit(() => {
          const ops = request.secretOperations || {};
          applySecret(["proxy", "password"], ops.proxyPassword);
          applySecret(["advanced", "twitter_manual_cookie"], ops.twitterManualCookie);
          merge(settings, copy(patch));
          const appearance = settings.appearance;
          appearance.theme = /^(omniget-desktop|amber-minimal|orange-soft|claude-amber|amber-hearth|amber-slate|orange-vivid)$/.test(appearance.preset) ? appearance.mode : appearance.preset;
        });
      }
      case "settings_reset_v3": {
        const request = args.request || {};
        checkRevision(request);
        const scope = request.scope;
        if (!scope || !["app", "section"].includes(scope.kind)) throw apiError("validation", "invalid_request", { fieldErrors: [{ path: "scope", code: "invalid_enum" }] });
        // Synthetic: restore the fixture's non-secret values of the scope; secrets/bridge/consent kept.
        return commit(() => {
          const groups = { general: ["start_with_system", "start_minimized", "exit_on_close"], appearance: ["appearance", "typography", "accessibility"], downloads: ["download"], ai: ["llm"], integrations: ["rpc", "world", "league", "music"], advanced: ["advanced"] };
          const names = scope.kind === "app" ? Object.values(groups).flat() : groups[scope.section] || [];
          for (const name of names) {
            if (name === "appearance") { Object.assign(settings.appearance, { mode: "system", preset: "omniget-desktop", theme: "system" }); continue; }
            if (name === "advanced") { const cookie = settings.advanced.twitter_manual_cookie; settings.advanced = { ...copy(defaults.advanced), twitter_manual_cookie: cookie }; continue; }
            if (name in defaults) settings[name] = copy(defaults[name]);
          }
        });
      }
      case "settings_retry_effect_v3": {
        const request = args.request || {};
        const effect = pendingEffects.find(item => item.id === request.effectId);
        if (!effect) return { outcome: "applied", snapshot: snapshot() };
        if (BigInt(request.targetRevision) < revision) return { outcome: "superseded", snapshot: snapshot() };
        pendingEffects = pendingEffects.filter(item => item.id !== request.effectId);
        emitChanged();
        return { outcome: "applied", snapshot: snapshot() };
      }
      case "settings_import_appearance_v3": {
        const request = args.request || {};
        if (request.source !== "omniget.workspace.appearance.v1") throw apiError("validation", "invalid_request", { fieldErrors: [{ path: "source", code: "invalid_value" }] });
        if (settings.appearance.migration_version >= 1) return { outcome: "already_imported", snapshot: snapshot() };
        return { outcome: "imported", snapshot: commit(() => { settings.appearance.migration_version = 1; }) };
      }
      // ---- legacy wrappers: same document, redacted DTO.
      case "get_settings": if (settingsState === "get-failed") throw new Error("HARNESS_INJECTED_FAILURE: get_settings"); return redact();
      case "update_settings": return commit(() => merge(settings, JSON.parse(args.partial))).settings;
      case "reset_settings": return commit(() => { const secrets = { password: settings.proxy.password, cookie: settings.advanced.twitter_manual_cookie, token: settings.bridge?.token }; settings = copy(initial); settings.proxy.password = secrets.password; settings.advanced.twitter_manual_cookie = secrets.cookie; settings.bridge.token = secrets.token; }).settings;
      // ---- domain fixtures (read-only unless noted)
      case "check_ytdlp_available": return true;
      case "check_cookie_error": return false;
      case "register_external_frontend": case "get_download_history": case "get_queue_state": case "get_download_queue": case "get_recovery_items": case "tool_mcp_auth_requests": return [];
      case "check_dependencies": return state === "empty" ? [] : [
        { name: "yt-dlp", installed: true, version: "fixture", source: "managed", path: "/synthetic/bin/yt-dlp", outdated: false },
        { name: "ffmpeg", installed: state !== "disabled", version: "fixture", source: "managed", path: "/synthetic/bin/ffmpeg", outdated: false },
      ];
      case "dependency_install_dir": return "/synthetic/dependencies/" + args.name;
      case "dependency_custom_path": return null;
      case "dependency_archived_versions": return [];
      case "browser_extension_status": return ["chrome", "firefox", "safari"].map(browser => ({ browser, supported: browser !== "safari", bundled_version: "fixture", installable: browser !== "safari" && state !== "disabled", install_hint_key: browser, store_url: null }));
      case "browser_extension_export": return { path: "/synthetic/extension", version: "fixture", browser: args.browser };
      case "get_bridge_info": return { enabled: true, port: state === "empty" ? 0 : 19999, token: "SYNTHETIC-NOT-A-CREDENTIAL", url: "http://127.0.0.1:19999/pair-synthetic" };
      case "profile_get": return state === "empty" ? null : profile;
      case "profile_set_nickname": profile.nickname = args.nick; return profile;
      case "profile_set_skin": profile.skin = { id: args.id, tint: args.tint }; return profile;
      case "ai_get_config": return ai;
      case "ai_set_config": ai = { ...ai, provider: args.provider, kind: "", model: args.model, local_base_url: args.localBaseUrl, has_openai_key: args.openaiKey === null ? ai.has_openai_key : !!args.openaiKey, has_anthropic_key: args.anthropicKey === null ? ai.has_anthropic_key : !!args.anthropicKey }; localStorage.setItem(key + ".ai", JSON.stringify(ai)); return ai;
      case "ai_history_list": return [];
      case "ai_test": return "SYNTHETIC_TEST_OK — no provider called";
      case "list_rules": return rules;
      case "save_rules": rules = copy(args.rules); return null;
      case "preview_rule_match": return null;
      case "content_store_stats": return { objects: 0, bytes_on_disk: 0, bytes_without_dedupe: 0, savings_measurable: true };
      case "get_portable_info": return { is_portable: false, data_dir: "/synthetic/omniget", macos_webview_notice: false };
      case "pet_capabilities": return { transparency: "supported", click_through: state !== "disabled", fallback_recommended: false, open: false, corner: "bottom-right", click_through_on: false };
      case "world_exists": return state !== "empty";
      case "cookies_list": return { registry: { buckets: {} }, cookies_dir: "/synthetic/cookies" };
      case "cookies_health": return { items: [], fresh_days: 7, expire_days: 30 };
      case "channels_list": return channels;
      case "channel_add": channels.push({ id: "fixture-added", url: args.url, title: "Canal adicionado no harness", added_at_ms: 1700000000000, last_checked_ms: null, enabled: true, auto_download: false, interval_minutes: 60 }); return null;
      case "channel_check_now": return 0;
      case "validate_output_path": return { limit: 1024, current: 52, reserve: 255, ok: true };
      case "usage_tray_view": return usageTray;
      case "usage_tray_set": Object.assign(usageTray.prefs, args.prefs || {}); return usageTray;
      default: {
        const extra = window.__SETTINGS_AUDIT_EXTRA__?.(command, args, { state, family });
        if (extra !== undefined) return extra;
        unexpected.push(command); throw new Error("HARNESS_UNHANDLED_COMMAND: " + command);
      }
    }
  };
  window.__SETTINGS_AUDIT__ = {
    evidenceKind: "frontend-harness-synthetic-ipc", behavior, mutatingCommands: MUTATING, isMutating,
    snapshot: () => ({ settings: redact(), revision: String(revision), ai: copy(ai), calls: copy(calls), unexpected: [...unexpected], mutations: calls.filter(call => isMutating(call.command)).map(call => call.command) }),
    emit: (event, payload) => { for (const item of listeners[event] || []) callbacks[item.callback]?.({ event, id: item.id, payload }); },
    secretSentinels: ["SYNTHETIC-SECRET-PROXY", "SYNTHETIC-SECRET-BRIDGE"],
  };
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args = {}) => {
      // Log names only. Fixtures are synthetic, but this keeps the recorder safe by construction.
      calls.push({ command, argumentKeys: Object.keys(args), at: Date.now(), mutating: isMutating(command) });
      if (behavior.reject.includes(command)) throw new Error("HARNESS_INJECTED_FAILURE: " + command);
      if (behavior.rejectWith[command]) throw copy(behavior.rejectWith[command]);
      if (behavior.hold.includes(command)) return new Promise(() => {});
      if (behavior.delay[command]) await new Promise(resolve => setTimeout(resolve, behavior.delay[command]));
      // Serialized like Tauri: ApiError objects reject as plain data, not Error instances.
      try { return copy(respond(command, args)); }
      catch (error) { if (error instanceof Error) throw error; throw copy(error); }
    },
    transformCallback: (callback, once = false) => { const id = ++callbackId; callbacks[id] = (...args) => { if (once) delete callbacks[id]; callback(...args); }; window["_" + id] = callbacks[id]; return id; },
    unregisterCallback: id => { delete callbacks[id]; delete window["_" + id]; },
    convertFileSrc: () => "data:,",
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.isTauri = true;
  // Legacy key present on purpose: with migration_version=1 the store must not import it.
  localStorage.setItem("omniget.workspace.appearance.v1", JSON.stringify({ preset: "amber-minimal", mode: theme }));
  Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
}

/**
 * Read-only fixtures for the contextual destinations outside /settings (roster, MCP,
 * music, Home templates, summaries, indicators). Installed as a second init script;
 * it only answers reads. Writes stay unknown (they reject and are recorded).
 */
export function installDestinationFixtures() {
  const bot = { id: "fixture-bot", name: "Fixture bot", role: "worker", system_prompt: "Synthetic bot for the settings harness.", model: { policy: "fixed", model: { provider: "local", model: "fixture-model" } }, tools: [], skills: [], runtime: { kind: "native" }, skin: null };
  const botProfile = { bot_id: bot.id, purpose: "Synthetic", instructions: "", capabilities: [], memory_policy: "ask", default_connection: null };
  const prefs = { enabled: false, edge: "top", along: {}, providers: [], notify_thresholds: false, thresholds: [80, 95], notify_reset: false, notify_pace: false, size: "regular", show_percent: true, show_pace: false, contrast: false };
  const reads = {
    get_updater_proxy_url: () => null,
    limits_strip_get_prefs: () => ({ prefs, open: false, providers: [] }),
    limits_strip_state: () => ({ rings: [] }),
    llm_roster_list: () => [bot],
    llm_mcp_list: () => [],
    tool_mcp_status: () => ({ enabled: false, bridge_enabled: true, port: 19998, url: "http://127.0.0.1:19998/mcp-synthetic", token: "SYNTHETIC-NOT-A-CREDENTIAL", tools: [], snippets: [] }),
    tool_mcp_clients: () => [],
    tool_mcp_media_budget: () => null,
    music_status: () => ({ port: 0, token: "", roots: [], track_count: 0, scanning: false, ffmpeg: true, download_dir: "/synthetic/music", analysis: [0, 0, false] }),
    music_tracks: () => [], music_history: () => [], music_playlists: () => [],
    yt_templates_list: () => [],
    tool_mcp_executors: () => [],
    tool_mcp_execution_grants_list: () => ({ grants: [], roots: [], derived: [] }),
    llm_skills_list: () => [],
    assist_bot_get: () => ({ agent: bot, profile: botProfile, bindings: [] }),
    assist_bot_capabilities: () => ({ bot_id: bot.id, runtime_kind: "native", runtime: { label: "Native", version: null, builtin_web: false, managed_tools: false, mcp_projection: false, limits: [] }, projectless: true, profile: botProfile, capabilities: [], skills: [], offered_tools: [], unavailable_grants: [] }),
    assist_bot_skill_reads: () => [],
    assist_memory_list: () => [],
    assist_memory_backups: () => ({ count: 0, newest_ms: null }),
    assist_reading_overview: () => ({ journeys: [], prefs: { bot_id: bot.id, region: "", subscriptions: [], access_kinds: [], only_confirmed: false, freshness_days: 30, updated_ms: 0 }, skill: null, skill_bound: false }),
    assist_learning_overview: () => ({ settings: { bot_id: bot.id, enabled: false, auto_promote_local: false, policy: { min_dev_cases: 3, min_holdout_cases: 2, min_dev_gain: 0.1, no_holdout_regression: true, unknown_blocks: true } }, observations: [], candidates: [], active: [], promotions: [], cases: {}, running_evals: [] }),
  };
  window.__SETTINGS_AUDIT_EXTRA__ = (command) => (command in reads ? reads[command]() : undefined);
}
