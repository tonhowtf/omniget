<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, type DependencyStatus } from "./settings-helpers";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Modal from "$components/music/Modal.svelte";
  import DependencyRow from "./DependencyRow.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import { refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";

  let { part }: { part: "extension" | "binaries" } = $props();

  let deps = $state<DependencyStatus[]>([]);
  let depsLoading = $state(false);
  let depsLoaded = $state(false);
  let depsError = $state(false);
  let installingDep = $state<string | null>(null);

  async function loadDeps() {
    if (depsLoading) return;
    depsLoading = true;
    depsError = false;
    try { deps = await invoke<DependencyStatus[]>("check_dependencies"); depsLoaded = true; }
    catch { depsError = true; }
    finally { depsLoading = false; }
  }

  async function onInstallDep(name: string) {
    if (installingDep) return;
    const wasInstalled = deps.find(d => d.name === name)?.installed ?? false;
    installingDep = name;
    try {
      const version = await invoke<string>("install_dependency", { name, force: wasInstalled });
      await loadDeps();
      await refreshYtdlpStatus();
      showToast("success", $t(wasInstalled ? "settings.dependencies.update_success" : "settings.dependencies.install_success", { name, version }));
    } catch { showToast("error", $t("common.error")); }
    finally { installingDep = null; }
  }

  const onRefresh = loadDeps;

  let settings = $derived(getSettings());

  type BrowserExtStatus = {
    browser: string;
    supported: boolean;
    bundled_version: string | null;
    installable: boolean;
    install_hint_key: string;
    store_url: string | null;
  };

  function hintFor(key: string): string {
    if (key === "chrome") return $t("settings.browser_ext.hint_chrome") as string;
    if (key === "firefox") return $t("settings.browser_ext.hint_firefox") as string;
    if (key === "safari") return $t("settings.browser_ext.hint_safari") as string;
    return "";
  }

  let extStatuses = $state<BrowserExtStatus[]>([]);
  let extensionsLoading = $state(true);
  let extensionsError = $state(false);
  let installTrigger: HTMLButtonElement | undefined;
  let busyBrowser = $state<string | null>(null);
  let installModal = $state<{ browser: string; path: string; hint: string } | null>(null);

  async function loadExtensions() {
    extensionsLoading = true;
    extensionsError = false;
    try { extStatuses = await invoke<BrowserExtStatus[]>("browser_extension_status"); }
    catch { extensionsError = true; }
    finally { extensionsLoading = false; }
  }
  onMount(() => {
    if (part === "extension") void loadExtensions();
    else void loadDeps();
  });

  async function exportExtension(browser: string, trigger: HTMLButtonElement) {
    if (busyBrowser) return;
    installTrigger = trigger;
    busyBrowser = browser;
    try {
      const res = await invoke<{ path: string; version: string | null; browser: string }>(
        "browser_extension_export",
        { browser },
      );
      const status = extStatuses.find((s) => s.browser === browser);
      installModal = {
        browser,
        path: res.path,
        hint: status ? hintFor(status.install_hint_key) : "",
      };
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      busyBrowser = null;
    }
  }

  async function openFolder(path: string) {
    try {
      await invoke("browser_extension_open_folder", { path });
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  function closeInstall() {
    installModal = null;
    void tick().then(() => installTrigger?.focus());
  }

  function browserLabel(b: string): string {
    if (b === "chrome") return "Chrome / Edge / Brave";
    if (b === "firefox") return "Firefox";
    if (b === "safari") return "Safari";
    return b;
  }
</script>

{#if settings && part === "extension"}
  <div class="ext-install" data-setting-key="settings.browser_ext.title" aria-busy={extensionsLoading}>
    <p class="muted">{$t("settings.browser_ext.description")}</p>
    {#if extensionsLoading}
      <p class="muted" role="status">{$t("settings.rebrand.state.loading")}</p>
    {:else if extensionsError}
      <SettingsEmptyState error title={$t("settings.dependencies.load_failed")} onRetry={() => void loadExtensions()} />
    {:else}
      {#each extStatuses as ext (ext.browser)}
        <SettingRow id={`browser-ext-${ext.browser}`} label={browserLabel(ext.browser)}
          help={ext.bundled_version ? $t("settings.browser_ext.bundled_version", { version: ext.bundled_version }) : hintFor(ext.install_hint_key)}>
          {#snippet control({ id, describedBy })}
            {#if ext.installable}
              <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={busyBrowser !== null}
                onclick={(e) => exportExtension(ext.browser, e.currentTarget)}>
                {busyBrowser === ext.browser ? $t("settings.browser_ext.extracting") : $t("settings.browser_ext.update_install")}
              </button>
            {:else if ext.store_url}
              <a {id} class="btn btn-secondary btn-sm" aria-describedby={describedBy} href={ext.store_url} target="_blank" rel="noreferrer">{$t("settings.browser_ext.open_store")}</a>
            {:else}
              <span {id} class="muted small">{$t("settings.browser_ext.unavailable")}</span>
            {/if}
          {/snippet}
        </SettingRow>
      {/each}
    {/if}
  </div>
{/if}

{#if settings && part === "binaries"}
  {#if depsLoading && !depsLoaded}
    <p class="muted" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
  {:else if depsError && !depsLoaded}
    <SettingsEmptyState error title={$t("settings.dependencies.load_failed")} onRetry={() => void loadDeps()} />
  {/if}
  {#if depsError && depsLoaded}
    <SettingsEmptyState error title={$t("settings.dependencies.load_failed")} onRetry={() => void loadDeps()} />
  {/if}
  {#if deps.length > 0}
    <div data-setting-key="settings.dependencies.title">
      <div class="deps-table-wrap">
        <table class="deps-table">
          <thead>
            <tr>
              <th scope="col">{$t('settings.dependencies.col_name')}</th>
              <th scope="col" data-setting-key="settings.dependencies.col_version">{$t('settings.dependencies.col_version')}</th>
              <th scope="col" data-setting-key="settings.dependencies.col_status">{$t('settings.dependencies.col_status')}</th>
              <th scope="col" class="deps-col-action">{$t('settings.dependencies.col_action')}</th>
            </tr>
          </thead>
          <tbody>
            {#each deps as dep (dep.name)}
              <DependencyRow
                name={dep.name}
                installed={dep.installed}
                version={dep.version}
                source={dep.source}
                path={dep.path}
                outdated={dep.outdated}
                busy={installingDep === dep.name}
                onInstall={() => onInstallDep(dep.name)}
                onAfterCustomFile={onRefresh}
              />
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}
{/if}

{#if installModal}
  <Modal title={$t("settings.browser_ext.install_title", { browser: browserLabel(installModal.browser) })} width={560} onclose={closeInstall}>
    <div class="body">
      <p>{$t("settings.browser_ext.extracted_to")}</p>
      <code class="path" dir="ltr">{installModal.path}</code>
      <button class="button" onclick={() => openFolder(installModal!.path)}>{$t("settings.browser_ext.open_folder")}</button>
      <h3>{$t("settings.browser_ext.install_steps")}</h3>
      <p class="hint">{installModal.hint}</p>
      <p class="hint">{$t("settings.browser_ext.reload_hint", { page: installModal.browser === "firefox" ? "about:debugging" : "chrome://extensions" })}</p>
    </div>
    <div class="foot"><button class="button" onclick={closeInstall}>{$t("common.close")}</button></div>
  </Modal>
{/if}

<style>
  .muted, .hint { color: var(--text-muted); font-size: var(--text-sm); line-height: var(--leading-base); margin: 0 0 var(--space-2); }
  .small { font-size: var(--text-xs); }
  .ext-install { display: flex; flex-direction: column; min-inline-size: 0; }
  .deps-table-wrap { overflow-x: auto; max-inline-size: 100%; }
  .body { display: flex; flex-direction: column; gap: var(--space-3); }
  .body h3 { margin: var(--space-2) 0 0; font-size: var(--text-base); }
  .body p { margin: 0; line-height: var(--leading-base); }
  .path { display: block; padding: var(--space-2) var(--space-3); background: var(--control-bg); color: var(--text); border: 1px solid var(--content-border); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: var(--text-sm); word-break: break-all; }
  .foot { padding-top: var(--space-4); display: flex; justify-content: flex-end; }
</style>
