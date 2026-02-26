<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t, locale, loadTranslations } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { completeOnboarding } from "$lib/stores/onboarding-store.svelte";
  import Mascot from "$components/mascot/Mascot.svelte";

  type DependencyStatus = {
    name: string;
    installed: boolean;
    version: string | null;
  };

  const TOTAL_STEPS = 4;

  let step = $state(1);
  let dialogEl = $state<HTMLDialogElement | null>(null);
  let deps = $state<DependencyStatus[]>([]);
  let installingDep = $state<string | null>(null);
  let settings = $derived(getSettings());

  $effect(() => {
    if (dialogEl && !dialogEl.open) {
      dialogEl.showModal();
    }
  });

  $effect(() => {
    if (step === 3) {
      loadDeps();
    }
  });

  async function loadDeps() {
    try {
      deps = await invoke<DependencyStatus[]>("check_dependencies");
    } catch {}
  }

  async function handleInstallDep(name: string) {
    installingDep = name;
    try {
      await invoke("install_dependency", { name });
      await loadDeps();
    } catch {} finally {
      installingDep = null;
    }
  }

  async function handleInstallAll() {
    const missing = deps.filter((d) => !d.installed);
    for (const dep of missing) {
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
    if (step < TOTAL_STEPS) {
      step++;
    }
  }

  function back() {
    if (step > 1) {
      step--;
    }
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
          <div class="language-row">
            <label class="language-label" for="onboarding-language">
              {$t("onboarding.language_label")}
            </label>
            <select
              id="onboarding-language"
              class="language-select"
              value={settings?.appearance.language ?? "en"}
              onchange={changeLanguage}
            >
              <option value="en">English</option>
              <option value="pt">Português</option>
            </select>
          </div>
        </div>
      {:else if step === 2}
        <div class="step step-folder">
          <div class="step-icon">
            <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M5 4h4l3 3h7a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V6a2 2 0 012-2z" />
            </svg>
          </div>
          <h2>{$t("onboarding.folder_title")}</h2>
          <p class="step-desc">{$t("onboarding.folder_desc")}</p>
          {#if settings}
            <div class="folder-info">
              <span class="folder-label">{$t("onboarding.folder_current")}</span>
              <span class="folder-path">{settings.download.default_output_dir}</span>
            </div>
            <button class="button folder-btn" onclick={chooseFolder}>
              {$t("onboarding.folder_change")}
            </button>
          {/if}
        </div>
      {:else if step === 3}
        <div class="step step-deps">
          <div class="step-icon">
            <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
              <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
            </svg>
          </div>
          <h2>{$t("onboarding.deps_title")}</h2>
          <p class="step-desc">{$t("onboarding.deps_desc")}</p>
          <div class="deps-list">
            {#each deps as dep}
              <div class="dep-row">
                <div class="dep-info">
                  <span class="dep-name">{dep.name}</span>
                  {#if dep.installed && dep.version}
                    <span class="dep-version dep-ok">v{dep.version}</span>
                  {:else}
                    <span class="dep-version dep-missing">{$t("settings.dependencies.not_found")}</span>
                  {/if}
                </div>
                {#if installingDep === dep.name}
                  <span class="dep-spinner"></span>
                {:else if !dep.installed}
                  <button class="button dep-btn" onclick={() => handleInstallDep(dep.name)}>
                    {$t("settings.dependencies.install")}
                  </button>
                {:else}
                  <svg class="dep-check" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M20 6L9 17l-5-5" />
                  </svg>
                {/if}
              </div>
            {/each}
          </div>
          {#if deps.some((d) => !d.installed) && installingDep === null}
            <button class="button install-all-btn" onclick={handleInstallAll}>
              {$t("onboarding.deps_install_all")}
            </button>
          {/if}
        </div>
      {:else if step === 4}
        <div class="step step-done">
          <Mascot emotion="idle" />
          <h2>{$t("onboarding.done_title")}</h2>
          <p class="step-desc">{$t("onboarding.done_desc")}</p>
        </div>
      {/if}
    </div>

    <div class="wizard-footer">
      <div class="dots">
        {#each Array(TOTAL_STEPS) as _, i}
          <span class="dot" class:active={i + 1 === step}></span>
        {/each}
      </div>
      <div class="footer-actions">
        {#if step > 1}
          <button class="button back-btn" onclick={back}>
            {$t("onboarding.back")}
          </button>
        {/if}
        {#if step < TOTAL_STEPS}
          <button class="button next-btn" onclick={next}>
            {$t("onboarding.next")}
          </button>
        {:else}
          <button class="button finish-btn" onclick={finish}>
            {$t("onboarding.finish")}
          </button>
        {/if}
      </div>
    </div>
  </div>
</dialog>

<style>
  .onboarding-dialog {
    border: none;
    border-radius: var(--border-radius);
    background: var(--popup-bg);
    color: var(--secondary);
    padding: 0;
    width: 90%;
    max-width: 440px;
    max-height: 90vh;
    max-height: 90dvh;
    animation: dialog-in 0.15s ease-out;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }

  .onboarding-dialog::backdrop {
    background: var(--dialog-backdrop);
    animation: backdrop-in 0.15s ease-out;
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
    min-height: 420px;
  }

  .wizard-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(var(--padding) + 4px) calc(var(--padding) + 4px) 0;
    flex-shrink: 0;
  }

  .step-indicator {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .skip-btn {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: calc(var(--border-radius) / 2);
  }

  @media (hover: hover) {
    .skip-btn:hover {
      color: var(--secondary);
      background: var(--button-elevated);
    }
  }

  .skip-btn:active {
    background: var(--button-elevated-press);
  }

  .skip-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .wizard-body {
    flex: 1;
    display: flex;
    justify-content: center;
    padding: calc(var(--padding) + 4px);
    overflow-y: auto;
    min-height: 0;
  }

  .step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: calc(var(--padding) / 2);
    width: 100%;
    margin: auto 0;
  }

  .step h2 {
    margin: 0;
    letter-spacing: -1px;
  }

  .step-desc {
    font-size: 14.5px;
    font-weight: 400;
    color: var(--gray);
    line-height: 1.6;
    max-width: 340px;
    margin: 0;
  }

  .language-row {
    display: flex;
    align-items: center;
    gap: var(--padding);
    margin-top: var(--padding);
    padding: calc(var(--padding) / 2) var(--padding);
    background: var(--button);
    border-radius: calc(var(--border-radius) / 2);
    box-shadow: var(--button-box-shadow);
    width: 100%;
  }

  .language-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--gray);
  }

  .language-select {
    margin-left: auto;
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 14.5px;
    font-weight: 500;
    font-family: var(--font-system);
    color: var(--secondary);
    background: var(--button-elevated);
    border: none;
    border-radius: calc(var(--border-radius) / 2);
    cursor: pointer;
    appearance: auto;
  }

  .language-select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .step-icon {
    color: var(--blue);
    margin-bottom: calc(var(--padding) / 2);
  }

  .step-icon :global(svg) {
    pointer-events: none;
  }

  .folder-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: var(--padding);
    padding: var(--padding);
    background: var(--button);
    border-radius: calc(var(--border-radius) / 2);
    width: 100%;
    box-shadow: var(--button-box-shadow);
  }

  .folder-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .folder-path {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .folder-btn {
    margin-top: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
  }

  .deps-list {
    display: flex;
    flex-direction: column;
    width: 100%;
    margin-top: var(--padding);
    background: var(--button);
    border-radius: calc(var(--border-radius) / 2);
    box-shadow: var(--button-box-shadow);
    overflow: hidden;
  }

  .dep-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--padding);
  }

  .dep-row + .dep-row {
    border-top: 1px solid var(--button-stroke);
  }

  .dep-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: left;
  }

  .dep-name {
    font-size: 14.5px;
    font-weight: 500;
  }

  .dep-version {
    font-size: 12.5px;
    font-weight: 500;
  }

  .dep-ok {
    color: var(--green);
  }

  .dep-missing {
    color: var(--red);
  }

  .dep-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    flex-shrink: 0;
  }

  .dep-spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    flex-shrink: 0;
  }

  .dep-check {
    color: var(--green);
    flex-shrink: 0;
    pointer-events: none;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .install-all-btn {
    margin-top: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
    background: var(--blue);
    color: #fff;
    box-shadow: none;
  }

  @media (hover: hover) {
    .install-all-btn:hover {
      opacity: 0.9;
      background: var(--blue);
    }
  }

  .install-all-btn:active {
    opacity: 0.85;
    background: var(--blue);
  }

  .wizard-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 calc(var(--padding) + 4px) calc(var(--padding) + 4px);
    flex-shrink: 0;
  }

  .dots {
    display: flex;
    gap: 6px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--button-elevated);
    transition: background 0.15s;
  }

  .dot.active {
    background: var(--blue);
  }

  .footer-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .back-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
    font-weight: 500;
  }

  .next-btn,
  .finish-btn {
    padding: calc(var(--padding) / 2) calc(var(--padding) * 1.5);
    font-size: 13px;
    font-weight: 500;
    background: var(--secondary);
    color: var(--primary);
    box-shadow: none;
  }

  @media (hover: hover) {
    .next-btn:hover,
    .finish-btn:hover {
      opacity: 0.9;
      background: var(--secondary);
    }
  }

  .next-btn:active,
  .finish-btn:active {
    opacity: 0.85;
    background: var(--secondary);
  }

  .next-btn:focus-visible,
  .finish-btn:focus-visible,
  .back-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
