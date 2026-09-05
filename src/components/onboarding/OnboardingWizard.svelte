<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t, locale, loadTranslations } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { completeOnboarding } from "$lib/stores/onboarding-store.svelte";
  import { refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import Mascot from "$components/mascot/Mascot.svelte";

  type DependencyStatus = {
    name: string;
    installed: boolean;
    version: string | null;
  };

  const TOTAL_STEPS = 3;
  const ENGINE_DEPS = new Set(["yt-dlp", "ffmpeg"]);

  const LANGUAGES: [string, string][] = [
    ["en", "English"],
    ["pt", "Português"],
    ["es", "Español"],
    ["ru", "Русский"],
    ["zh", "中文"],
    ["zh-TW", "繁體中文"],
    ["ja", "日本語"],
    ["it", "Italiano"],
    ["fr", "Français"],
    ["el", "Ελληνικά"],
    ["fa", "فارسی"],
  ];

  let step = $state(1);
  let dialogEl = $state<HTMLDialogElement | null>(null);
  let deps = $state<DependencyStatus[]>([]);
  let depsLoaded = $state(false);
  let installingDep = $state<string | null>(null);
  let settings = $derived(getSettings());

  let missingDeps = $derived(deps.filter((d) => !d.installed));
  let allInstalled = $derived(depsLoaded && missingDeps.length === 0);
  let progress = $derived(Math.round((step / TOTAL_STEPS) * 100));

  $effect(() => {
    if (dialogEl && !dialogEl.open) {
      dialogEl.showModal();
    }
  });

  $effect(() => {
    if (step === 2) {
      loadDeps();
    }
  });

  async function loadDeps() {
    try {
      const all = await invoke<DependencyStatus[]>("check_dependencies");
      deps = all.filter((d) => ENGINE_DEPS.has(d.name));
      depsLoaded = true;
    } catch {}
  }

  async function handleInstallDep(name: string) {
    installingDep = name;
    try {
      await invoke("install_dependency", { name });
      await loadDeps();
      await refreshYtdlpStatus();
    } catch {} finally {
      installingDep = null;
    }
  }

  async function handleInstallAll() {
    for (const dep of missingDeps) {
      await handleInstallDep(dep.name);
    }
  }

  async function changeLanguage(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await updateSettings({ appearance: { language: value } });
    await loadTranslations(value, "/");
    locale.set(value);
  }

  async function chooseFolder() {
    const selected = await open({
      directory: true,
      title: $t("onboarding.folder_title"),
    });
    if (selected) {
      await updateSettings({ download: { default_output_dir: selected } });
    }
  }

  function next() {
    if (step < TOTAL_STEPS) step++;
  }

  function back() {
    if (step > 1) step--;
  }

  async function finish() {
    await completeOnboarding();
  }

  async function skip() {
    await completeOnboarding();
  }
</script>

<dialog
  bind:this={dialogEl}
  class="onboarding-dialog"
  oncancel={(e) => e.preventDefault()}
>
  <div class="wizard">
    <div class="wizard-header">
      <div class="wizard-progress" role="progressbar" aria-valuenow={progress} aria-valuemin={0} aria-valuemax={100} aria-label={$t("onboarding.step_of", { current: step, total: TOTAL_STEPS })}>
        <div class="wizard-progress-fill" style:width="{progress}%"></div>
      </div>
      <span class="step-indicator">
        {$t("onboarding.step_of", { current: step, total: TOTAL_STEPS })}
      </span>
      <button class="skip-btn" onclick={skip}>
        {$t("onboarding.skip")}
      </button>
    </div>

    <div class="wizard-body">
      {#if step === 1}
        <div class="step step-welcome">
          <Mascot emotion="idle" />
          <h2>{$t("onboarding.welcome_title")}</h2>
          <p class="step-desc">{$t("onboarding.welcome_desc")}</p>
          <div class="pref-list">
            <label class="pref-row" for="onboarding-language">
              <span class="pref-label">{$t("onboarding.language_label")}</span>
              <select
                id="onboarding-language"
                class="pref-select"
                value={settings?.appearance.language ?? "en"}
                onchange={changeLanguage}
              >
                {#each LANGUAGES as [code, name] (code)}
                  <option value={code}>{name}</option>
                {/each}
              </select>
            </label>
            <div class="pref-row">
              <span class="pref-label">{$t("onboarding.theme_label")}</span>
              <div class="segmented" role="radiogroup" aria-label={$t("onboarding.theme_label")}>
                {#each [["system", "onboarding.theme_system"], ["light", "onboarding.theme_light"], ["dark", "onboarding.theme_dark"]] as [id, key] (id)}
                  <button
                    type="button"
                    class="segmented-btn"
                    class:active={(settings?.appearance.theme ?? "system") === id}
                    role="radio"
                    aria-checked={(settings?.appearance.theme ?? "system") === id}
                    onclick={() => updateSettings({ appearance: { theme: id } })}
                  >{$t(key)}</button>
                {/each}
              </div>
            </div>
          </div>
        </div>
      {:else if step === 2}
        <div class="step step-deps">
          <h2>{$t("onboarding.deps_title")}</h2>
          <p class="step-desc">{$t("onboarding.deps_desc")}</p>
          <div class="deps-list">
            {#each deps as dep (dep.name)}
              <div class="dep-row" data-installed={dep.installed}>
                <span class="dep-dot" aria-hidden="true"></span>
                <span class="dep-name">{dep.name}</span>
                {#if installingDep === dep.name}
                  <span class="spinner dep-spinner"></span>
                {:else if dep.installed}
                  <span class="dep-version">{dep.version ? `v${dep.version}` : $t("settings.dependencies.status_installed")}</span>
                {:else}
                  <button class="btn btn-secondary btn-sm" onclick={() => handleInstallDep(dep.name)}>
                    {$t("settings.dependencies.install")}
                  </button>
                {/if}
              </div>
            {/each}
            {#if !depsLoaded}
              <div class="dep-row"><span class="spinner dep-spinner"></span></div>
            {/if}
          </div>
          {#if allInstalled}
            <p class="deps-ready">{$t("onboarding.deps_ready")}</p>
          {:else if missingDeps.length > 0 && installingDep === null}
            <button class="btn btn-primary btn-lg" onclick={handleInstallAll}>
              {$t("onboarding.deps_install_all")}
            </button>
          {/if}
        </div>
      {:else}
        <div class="step step-done">
          <Mascot emotion="complete" />
          <h2>{$t("onboarding.done_title")}</h2>
          <p class="step-desc">{$t("onboarding.done_desc")}</p>
          {#if settings}
            <div class="folder-row">
              <span class="folder-label">{$t("onboarding.folder_current")}</span>
              <span class="folder-path" title={settings.download.default_output_dir}>{settings.download.default_output_dir}</span>
              <button class="btn btn-ghost btn-sm" onclick={chooseFolder}>
                {$t("onboarding.folder_change")}
              </button>
            </div>
          {/if}
          <p class="step-note">{$t("onboarding.more_later")}</p>
        </div>
      {/if}
    </div>

    <div class="wizard-footer">
      {#if step > 1}
        <button class="btn btn-ghost" onclick={back}>
          {$t("onboarding.back")}
        </button>
      {:else}
        <span></span>
      {/if}
      {#if step < TOTAL_STEPS}
        <button class="btn btn-lg" class:btn-primary={!(step === 2 && missingDeps.length > 0)} class:btn-secondary={step === 2 && missingDeps.length > 0} onclick={next}>
          {$t("onboarding.next")}
        </button>
      {:else}
        <button class="btn btn-primary btn-lg" onclick={finish}>
          {$t("onboarding.finish")}
        </button>
      {/if}
    </div>
  </div>
</dialog>

<style>
  .onboarding-dialog {
    margin: auto;
    border: none;
    border-radius: var(--radius-xl);
    background: var(--popup-bg);
    color: var(--text);
    padding: 0;
    width: 90%;
    max-width: 460px;
    max-height: 90vh;
    max-height: 90dvh;
    animation: dialog-in var(--duration-slow) var(--ease-spring);
    box-shadow: var(--elev-3);
    overflow: hidden;
    transform-origin: top center;
  }

  .onboarding-dialog::backdrop {
    background: var(--dialog-backdrop);
    animation: backdrop-in var(--duration-base) var(--ease-out);
  }

  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(8px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  @keyframes backdrop-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .wizard {
    display: flex;
    flex-direction: column;
    min-height: 440px;
  }

  .wizard-header {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-5) 0;
    flex-shrink: 0;
  }

  .wizard-progress {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: var(--fill-1);
  }

  .wizard-progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width var(--duration-slow) var(--ease-out);
  }

  .step-indicator {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .skip-btn {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-dim);
    background: none;
    border: none;
    cursor: pointer;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
  }

  @media (hover: hover) {
    .skip-btn:hover {
      color: var(--text);
      background: var(--fill-1);
    }
  }

  .skip-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .wizard-body {
    flex: 1;
    display: flex;
    justify-content: center;
    padding: var(--space-5);
    overflow-y: auto;
    min-height: 0;
  }

  .step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-2);
    width: 100%;
    margin: auto 0;
  }

  .step h2 {
    margin: var(--space-2) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .step-desc {
    font-size: var(--text-base);
    color: var(--text-muted);
    line-height: var(--leading-base);
    max-width: 340px;
    margin: 0;
  }

  .step-note {
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: var(--leading-sm);
    max-width: 340px;
    margin: var(--space-2) 0 0;
  }

  .pref-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    margin-top: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .pref-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: var(--row-base);
    padding: var(--space-2) var(--space-3);
    position: relative;
  }

  .pref-row + .pref-row::before {
    content: "";
    position: absolute;
    top: 0;
    left: var(--space-3);
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }

  .pref-label {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
  }

  .pref-select {
    max-width: 200px;
  }

  .pref-select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .deps-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    margin-top: var(--space-4);
    padding: 0;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .dep-row + .dep-row {
    border-top: var(--hairline) solid var(--separator);
  }

  .dep-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 40px;
    padding: 0 var(--space-3);
  }

  .dep-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--text-faint);
    flex-shrink: 0;
  }

  .dep-row[data-installed="true"] .dep-dot {
    background: var(--success);
  }

  .dep-name {
    flex: 1;
    text-align: left;
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
    font-family: var(--font-mono);
  }

  .dep-version {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .dep-spinner {
    width: 14px;
    height: 14px;
  }

  .deps-ready {
    margin: var(--space-2) 0 0;
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--success);
  }

  .step-deps .btn-primary {
    margin-top: var(--space-3);
  }

  .folder-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    margin-top: var(--space-3);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
  }

  .folder-label {
    flex-shrink: 0;
    color: var(--text-dim);
  }

  .folder-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
    font-family: var(--font-mono);
    color: var(--text);
  }

  .wizard-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-5);
    flex-shrink: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .onboarding-dialog,
    .onboarding-dialog::backdrop {
      animation: none;
    }

    .wizard-progress-fill {
      transition: none;
    }
  }
</style>
