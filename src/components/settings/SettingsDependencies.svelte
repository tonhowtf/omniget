<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, type DependencyStatus } from "./settings-helpers";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Modal from "$components/music/Modal.svelte";
  import DependencyRow from "./DependencyRow.svelte";

  let { deps = [], installingDep = null, onInstallDep, onRefresh }: {
    deps?: DependencyStatus[];
    installingDep?: string | null;
    onInstallDep: (name: string) => void;
    onRefresh?: () => void | Promise<void>;
  } = $props();

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
  onMount(() => { void loadExtensions(); });

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

{#if settings}
  <section class="section">
    <h5 class="section-title">{$t("settings.browser_ext.title")}</h5>
    <p class="muted">{$t("settings.browser_ext.description")}</p>
    {#if extensionsLoading}<p role="status">{$t("common.loading")}</p>{/if}
    {#if extensionsError}<div role="alert"><p>{$t("settings.dependencies.load_failed")}</p><button class="button" onclick={loadExtensions}>{$t("settings.retry")}</button></div>{/if}
    <div class="card">
      {#each extStatuses as ext, i (ext.browser)}
        {#if i > 0}<div class="divider"></div>{/if}
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{browserLabel(ext.browser)}</span>
            <span class="setting-path">
              {#if ext.bundled_version}
                {$t("settings.browser_ext.bundled_version", { version: ext.bundled_version })}
              {:else}
                {hintFor(ext.install_hint_key)}
              {/if}
            </span>
          </div>
          {#if ext.installable}
            <button
              class="button"
              disabled={busyBrowser !== null}
              onclick={(e) => exportExtension(ext.browser, e.currentTarget)}
            >
              {busyBrowser === ext.browser
                ? $t("settings.browser_ext.extracting")
                : $t("settings.browser_ext.update_install")}
            </button>
          {:else if ext.store_url}
            <a class="button" href={ext.store_url} target="_blank" rel="noreferrer">{$t("settings.browser_ext.open_store")}</a>
          {:else}
            <span class="muted small">{$t("settings.browser_ext.unavailable")}</span>
          {/if}
        </div>
      {/each}
    </div>
  </section>

  {#if deps.length > 0}
    <section class="section">
      <div class="settings-section-head section-title">
        <h5 class="section-title">{$t('settings.dependencies.title')}</h5>
      </div>
      <div class="deps-table-wrap">
        <table class="deps-table">
          <thead>
            <tr>
              <th scope="col">{$t('settings.dependencies.col_name')}</th>
              <th scope="col">{$t('settings.dependencies.col_version')}</th>
              <th scope="col">{$t('settings.dependencies.col_status')}</th>
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
    </section>
  {/if}
{/if}

{#if installModal}
  <Modal title={$t("settings.browser_ext.install_title", { browser: browserLabel(installModal.browser) })} width={560} onclose={closeInstall}>
    <div class="body">
      <p>{$t("settings.browser_ext.extracted_to")}</p>
      <code class="path">{installModal.path}</code>
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
  .body { display: flex; flex-direction: column; gap: var(--space-3); }
  .body h3 { margin: var(--space-2) 0 0; font-size: var(--text-base); }
  .body p { margin: 0; line-height: var(--leading-base); }
  .path { display: block; padding: var(--space-2) var(--space-3); background: var(--control-bg); color: var(--text); border: 1px solid var(--content-border); border-radius: var(--radius-sm); font-family: var(--font-mono); font-size: var(--text-sm); word-break: break-all; }
  .foot { padding-top: var(--space-4); display: flex; justify-content: flex-end; }
</style>
