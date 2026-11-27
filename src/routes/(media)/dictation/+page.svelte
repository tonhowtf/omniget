<script lang="ts">
  import { onMount } from "svelte";
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/dictation/api";
  import { acceptState, errorKey, formatBytes, insertAt, isShortcutError, shortcutErrorKey, splitError, type ErrorAction } from "$lib/dictation/logic";
  import type { Device, Entry, ModelProgress, Settings, StateEvent, Status } from "$lib/dictation/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import StateCard from "#components/dictation/StateCard.svelte";
  import HistoryList from "#components/dictation/HistoryList.svelte";
  import SettingsPanel from "#components/dictation/SettingsPanel.svelte";
  import Icon from "#components/dictation/Icon.svelte";

  const BARS = 40;

  let status = $state<Status | null>(null);
  let loadError = $state<string | null>(null);
  let settings = $state<Settings | null>(null);
  let live = $state<StateEvent>({ phase: "idle", session: 0, source: "page", started_at: null, max_ms: 0, entry: null, outcome: null, error: null, notice: null });
  let levels = $state<number[]>(Array(BARS).fill(0));
  let elapsed = $state(0);
  let history = $state<Entry[]>([]);
  let historyLoading = $state(true);
  let devices = $state<Device[]>([]);
  let devicesLoading = $state(false);
  let devicesError = $state<string | null>(null);
  let shortcutError = $state<string | null>(null);
  let shortcutSaving = $state(false);
  let saveError = $state<string | null>(null);
  let savedFlash = $state(false);
  let modelProgress = $state<ModelProgress | null>(null);
  let actionBusy = $state(false);
  let testText = $state("");
  let testArea: HTMLTextAreaElement | undefined = $state();
  let previewInput = $state("");
  let previewOutput = $state("");

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveChain: Promise<unknown> = Promise.resolve();
  let flashTimer: ReturnType<typeof setTimeout> | null = null;
  let previewTimer: ReturnType<typeof setTimeout> | null = null;
  let tick: ReturnType<typeof setInterval> | null = null;

  let busy = $derived(live.phase === "listening" || live.phase === "transcribing");

  let blocked = $derived.by((): string | null => {
    if (!status) return null;
    if (!status.ffmpeg) return $t("dictation.err_ffmpeg_missing_body") as string;
    if (!status.runtime_installed && !status.runtime_downloadable) return $t("dictation.err_asr_unavailable_body") as string;
    if (!status.model_ready) return $t("dictation.setup_model_body") as string;
    return null;
  });

  let needsSetup = $derived(!!status && (!status.ffmpeg || !status.model_ready));
  let selectedModel = $derived(status && settings ? (status.models.find((m) => m.id === settings!.model) ?? null) : null);
  let setupPercent = $derived(modelProgress ? Math.round(modelProgress.fraction * 100) : 0);

  function message(raw: unknown, shortcut = false): string {
    const { code, detail } = splitError(api.errorText(raw));
    if (shortcut && isShortcutError(code)) return $t(shortcutErrorKey(code)) as string;
    if (isShortcutError(code)) return $t(shortcutErrorKey(code)) as string;
    const key = errorKey(code);
    const body = $t(`${key}_body`) as string;
    return code === "UNKNOWN" && detail ? `${body} (${detail})` : body;
  }

  function applyStatus(next: Status) {
    status = next;
    settings = structuredClone(next.settings);
    shortcutError = next.shortcut.error && next.settings.enabled ? ($t(shortcutErrorKey(next.shortcut.error)) as string) : null;
  }

  async function load() {
    loadError = null;
    try {
      const s = await api.status();
      applyStatus(s);
      if (acceptState(null, s.state)) setLive(s.state);
    } catch (e) {
      loadError = message(e);
    }
  }

  async function loadHistory() {
    historyLoading = true;
    try {
      history = await api.history();
    } catch {
      history = [];
    } finally {
      historyLoading = false;
    }
  }

  async function loadDevices() {
    devicesLoading = true;
    devicesError = null;
    try {
      devices = await api.devices();
    } catch (e) {
      devices = [];
      devicesError = message(e);
    } finally {
      devicesLoading = false;
    }
  }

  function flashSaved() {
    savedFlash = true;
    if (flashTimer) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (savedFlash = false), 1400);
  }

  function persist(next: Settings): Promise<boolean> {
    const run = saveChain.then(() => save(next));
    saveChain = run;
    return run;
  }

  async function save(next: Settings): Promise<boolean> {
    saveError = null;
    try {
      const res = await api.saveSettings(next);
      status = res;
      shortcutError = null;
      flashSaved();
      return true;
    } catch (e) {
      const { code } = splitError(api.errorText(e));
      if (isShortcutError(code)) shortcutError = message(e, true);
      else saveError = message(e);
      return false;
    }
  }

  function change(patch: Partial<Settings>) {
    if (!settings) return;
    settings = { ...settings, ...patch };
    if (saveTimer) clearTimeout(saveTimer);
    const snapshot = $state.snapshot(settings) as Settings;
    const immediate = !("replacements" in patch);
    saveTimer = setTimeout(() => void persist(snapshot), immediate ? 0 : 450);
  }

  async function toggleShortcut(on: boolean) {
    if (!settings) return;
    shortcutSaving = true;
    settings.enabled = on;
    const ok = await persist($state.snapshot(settings) as Settings);
    if (!ok && settings && settings.enabled === on) settings.enabled = !on;
    shortcutSaving = false;
  }

  async function pickShortcut(combo: string) {
    if (!settings) return;
    shortcutSaving = true;
    shortcutError = null;
    try {
      const check = await api.checkShortcut(combo);
      if (check.error || !check.normalized) {
        shortcutError = $t(shortcutErrorKey(check.error ?? "UNKNOWN_KEY")) as string;
        return;
      }
      const previous = settings.shortcut;
      settings.shortcut = check.normalized;
      const ok = await persist($state.snapshot(settings) as Settings);
      if (!ok && settings && settings.shortcut === check.normalized) settings.shortcut = previous;
    } catch (e) {
      shortcutError = message(e, true);
    } finally {
      shortcutSaving = false;
    }
  }

  function setLive(next: StateEvent) {
    const wasListening = live.phase === "listening";
    live = next;
    if (next.phase === "listening") {
      if (!wasListening) levels = Array(BARS).fill(0);
      startTick();
    } else {
      stopTick();
    }
  }

  function startTick() {
    stopTick();
    const update = () => {
      elapsed = live.started_at ? Date.now() - live.started_at : 0;
    };
    update();
    tick = setInterval(update, 250);
  }

  function stopTick() {
    if (tick) clearInterval(tick);
    tick = null;
  }

  function onState(next: StateEvent) {
    if (!acceptState(live, next)) return;
    setLive(next);
    if (next.phase === "done" && next.entry) {
      const entry = next.entry;
      if (settings?.save_history && !history.some((h) => h.id === entry.id)) history = [entry, ...history].slice(0, 200);
      if (next.outcome === "inserted") insertIntoTest(entry.text);
    }
  }

  function insertIntoTest(text: string) {
    const el = testArea;
    const start = el?.selectionStart ?? testText.length;
    const end = el?.selectionEnd ?? testText.length;
    const res = insertAt(testText, text, start, end);
    testText = res.value;
    queueMicrotask(() => {
      if (!el) return;
      el.focus();
      el.setSelectionRange(res.caret, res.caret);
    });
  }

  async function startPage() {
    if (busy || blocked) return;
    try {
      const ev = await api.start();
      onState(ev);
    } catch (e) {
      const { code } = splitError(api.errorText(e));
      if (code === "BUSY") return;
      if (live.phase !== "error") saveError = message(e);
    }
  }

  async function stopPage() {
    try {
      await api.stop();
    } catch (e) {
      saveError = message(e);
    }
  }

  async function cancelPage() {
    try {
      await api.cancel();
    } catch (e) {
      saveError = message(e);
    }
  }

  async function downloadModel(id: string | null) {
    if (!status || status.installing) return;
    status = { ...status, installing: true };
    modelProgress = { model: id ?? "", stage: "model", fraction: 0 };
    saveError = null;
    try {
      applyStatus(await api.installModel(id));
    } catch (e) {
      saveError = message(e);
      status = { ...status, installing: false };
    } finally {
      modelProgress = null;
    }
  }

  async function runAction(action: ErrorAction) {
    actionBusy = true;
    try {
      if (action === "download_model") await downloadModel(settings?.model ?? null);
      else if (action === "open_accessibility") await api.openPrivacy("accessibility");
      else if (action === "open_automation") await api.openPrivacy("automation");
      else if (action === "open_microphone") await api.openPrivacy("microphone");
      else if (action === "retry") await startPage();
    } catch (e) {
      saveError = message(e);
    } finally {
      actionBusy = false;
    }
  }

  async function copyEntry(entry: Entry): Promise<boolean> {
    try {
      await api.copy(entry.text);
      return true;
    } catch (e) {
      saveError = message(e);
      return false;
    }
  }

  async function deleteEntry(entry: Entry) {
    const before = history;
    history = history.filter((h) => h.id !== entry.id);
    try {
      history = await api.deleteEntry(entry.id);
    } catch (e) {
      history = before;
      saveError = message(e);
    }
  }

  async function clearAll() {
    const before = history;
    history = [];
    try {
      await api.clearHistory();
    } catch (e) {
      history = before;
      saveError = message(e);
    }
  }

  $effect(() => {
    const input = previewInput;
    const snap = settings ? ($state.snapshot(settings) as Settings) : null;
    const fallback = history[0]?.language || $locale || null;
    if (previewTimer) clearTimeout(previewTimer);
    if (!input.trim() || !snap) {
      previewOutput = "";
      return;
    }
    previewTimer = setTimeout(() => {
      api
        .preview(input, snap, fallback)
        .then((out) => (previewOutput = out))
        .catch(() => (previewOutput = ""));
    }, 200);
  });

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && live.phase === "listening") {
      e.preventDefault();
      void cancelPage();
    }
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };
    void api.onState(onState).then(keep).catch(() => {});
    void api
      .onLevel((e) => {
        if (e.session !== live.session || live.phase !== "listening") return;
        levels = [...levels.slice(1), e.level];
      })
      .then(keep)
      .catch(() => {});
    void api.onModel((p) => (modelProgress = p)).then(keep).catch(() => {});
    void load();
    void loadHistory();
    void loadDevices();
    const onFocus = () => {
      if (!busy) void api.status().then((s) => status && (status = { ...s, settings: status.settings })).catch(() => {});
    };
    window.addEventListener("focus", onFocus);
    return () => {
      disposed = true;
      window.removeEventListener("focus", onFocus);
      cleanups.forEach((fn) => fn());
      stopTick();
      if (saveTimer) clearTimeout(saveTimer);
      if (previewTimer) clearTimeout(previewTimer);
      if (flashTimer) clearTimeout(flashTimer);
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} />

<section class="dc" aria-busy={busy}>
  <header class="dc-head">
    <NavIcon icon="dictation" size={56} />
    <div class="dc-head-copy">
      <h1>{$t("dictation.title")}</h1>
      <p class="dc-subtitle">{$t("dictation.subtitle")}</p>
    </div>
    <span class="dc-saved" class:show={savedFlash} role="status" aria-live="polite">
      {#if savedFlash}<Icon name="done" size={12} /> {$t("dictation.saved")}{/if}
    </span>
  </header>

  {#if saveError}
    <div class="notice notice-danger" role="alert">
      <Icon name="error" size={16} />
      <div class="notice-body"><p class="dc-notice-text">{saveError}</p></div>
      <button type="button" class="btn btn-ghost btn-sm btn-icon" onclick={() => (saveError = null)} aria-label={$t("dictation.dismiss") as string}><Icon name="close" size={12} /></button>
    </div>
  {/if}

  {#if loadError}
    <div class="dc-loading">
      <Icon name="error" size={22} />
      <p>{loadError}</p>
      <button type="button" class="btn btn-primary" onclick={load}>{$t("dictation.reload")}</button>
    </div>
  {:else if !status || !settings}
    <div class="dc-loading" aria-live="polite">
      <span class="spinner" aria-hidden="true"></span>
      <p>{$t("dictation.loading")}</p>
    </div>
  {:else}
    {#if needsSetup}
      <section class="dc-setup" aria-labelledby="dc-setup-title">
        <h2 id="dc-setup-title">{$t("dictation.setup_title")}</h2>
        <ol class="dc-steps">
          <li class="dc-step" class:ok={status.ffmpeg}>
            <Icon name={status.ffmpeg ? "done" : "warning"} size={16} />
            <span class="dc-step-copy">
              <span class="field-label">{$t("dictation.setup_ffmpeg")}</span>
              <span class="field-hint">{status.ffmpeg ? $t("dictation.setup_ffmpeg_ok") : $t("dictation.err_ffmpeg_missing_body")}</span>
            </span>
          </li>
          <li class="dc-step" class:ok={status.model_ready}>
            <Icon name={status.model_ready ? "done" : "download"} size={16} />
            <span class="dc-step-copy">
              <span class="field-label">{$t("dictation.setup_model", { size: selectedModel ? formatBytes(selectedModel.bytes) : "" })}</span>
              {#if status.installing && modelProgress}
                <span class="dc-setup-progress">
                  <span class="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={setupPercent} aria-label={$t("dictation.model_downloading") as string}>
                    <span class="progress-fill" style:width="{setupPercent}%"></span>
                  </span>
                  <span class="field-hint">{modelProgress.stage === "runtime" ? $t("dictation.runtime_downloading", { percent: setupPercent }) : $t("dictation.model_downloading_pct", { percent: setupPercent })}</span>
                </span>
              {:else if !status.runtime_installed && !status.runtime_downloadable}
                <span class="field-hint">{$t("dictation.err_asr_unavailable_body")}</span>
              {:else}
                <span class="field-hint">{status.model_ready ? $t("dictation.model_ready") : $t("dictation.setup_model_body")}</span>
              {/if}
            </span>
            {#if !status.model_ready && (status.runtime_installed || status.runtime_downloadable)}
              <button type="button" class="btn btn-primary" disabled={status.installing} onclick={() => downloadModel(settings?.model ?? null)}>
                {status.installing ? $t("dictation.model_downloading") : $t("dictation.model_download")}
              </button>
            {/if}
          </li>
        </ol>
      </section>
    {/if}

    <div class="dc-grid">
      <div class="dc-main">
        <StateCard
          state={live}
          platform={status.platform}
          shortcut={settings.shortcut}
          mode={settings.mode}
          shortcutOn={settings.enabled && status.shortcut.registered}
          {blocked}
          {levels}
          {elapsed}
          {actionBusy}
          onstart={startPage}
          onstop={stopPage}
          oncancel={cancelPage}
          onaction={runAction}
        />

        <section class="dc-test" aria-labelledby="dc-test-title">
          <div class="dc-test-head">
            <h2 id="dc-test-title">{$t("dictation.test_title")}</h2>
            {#if testText}
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => (testText = "")}>{$t("dictation.test_clear")}</button>
            {/if}
          </div>
          <textarea
            bind:this={testArea}
            bind:value={testText}
            class="input dc-test-area"
            rows="4"
            placeholder={$t("dictation.test_ph") as string}
            aria-describedby="dc-test-hint"
          ></textarea>
          <p id="dc-test-hint" class="field-hint">{$t("dictation.test_hint")}</p>
        </section>

        <HistoryList
          entries={history}
          loading={historyLoading}
          saveHistory={settings.save_history}
          oncopy={copyEntry}
          ondelete={deleteEntry}
          onclear={clearAll}
        />
      </div>

      <aside class="dc-side" aria-label={$t("dictation.settings") as string}>
        <SettingsPanel
          {settings}
          {status}
          {devices}
          {devicesLoading}
          {devicesError}
          {shortcutError}
          {shortcutSaving}
          locked={busy}
          {modelProgress}
          bind:previewInput
          {previewOutput}
          onchange={change}
          ontoggleshortcut={toggleShortcut}
          onpickshortcut={pickShortcut}
          onrecordstart={() => void api.pauseShortcut(true).catch(() => {})}
          onrecordend={() => void api.pauseShortcut(false).catch(() => {})}
          onrefreshdevices={loadDevices}
          ondownloadmodel={(id) => downloadModel(id)}
        />
      </aside>
    </div>
  {/if}
</section>

<style>
  .dc {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .dc-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .dc-head-copy {
    flex: 1;
    min-width: 0;
  }

  .dc-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .dc-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .dc-saved {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 64px;
    justify-content: flex-end;
    font-size: var(--text-xs);
    color: var(--success);
    opacity: 0;
    transition: opacity var(--duration-base) var(--ease-out);
  }

  .dc-saved.show {
    opacity: 1;
  }

  .dc-notice-text {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .dc-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    min-height: 240px;
    justify-content: center;
    color: var(--text-dim);
    text-align: center;
  }

  .dc-loading p {
    margin: 0;
    max-width: 420px;
  }

  .dc-setup {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .dc-setup h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .dc-steps {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .dc-step {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    color: var(--warning);
  }

  .dc-step.ok {
    color: var(--success);
  }

  .dc-step-copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .dc-setup-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-width: 360px;
    margin-top: var(--space-1);
  }

  .dc-setup-progress .progress {
    display: block;
  }

  .dc-setup-progress .progress-fill {
    display: block;
  }

  .dc-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }

  @media (min-width: 900px) {
    .dc-grid {
      grid-template-columns: minmax(0, 1fr) 340px;
    }
  }

  .dc-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
  }

  .dc-test {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .dc-test-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: var(--control-h);
  }

  .dc-test-head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .dc-test-area {
    height: auto;
    min-height: 96px;
    padding: var(--space-2) var(--space-3);
    line-height: var(--leading-base);
    resize: vertical;
  }

  .dc-test p {
    margin: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .dc-saved {
      transition: none;
    }
  }
</style>
