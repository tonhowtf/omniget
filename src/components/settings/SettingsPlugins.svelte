<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, type DependencyStatus } from "./settings-helpers";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import DependencyRow from "./DependencyRow.svelte";

  let { deps = [], installingDep = null, onInstallDep, onRefresh }: {
    deps?: DependencyStatus[];
    installingDep?: string | null;
    onInstallDep: (name: string, variant: string | null) => void;
    onRefresh?: () => void | Promise<void>;
  } = $props();

  let settings = $derived(getSettings());

  type BrowserExtStatus = {
    browser: string;
    supported: boolean;
    bundled_version: string | null;
    installable: boolean;
    install_hint: string;
    store_url: string | null;
  };

  let extStatuses = $state<BrowserExtStatus[]>([]);
  let busyBrowser = $state<string | null>(null);
  let installModal = $state<{ browser: string; path: string; hint: string } | null>(null);

  onMount(async () => {
    try {
      extStatuses = await invoke<BrowserExtStatus[]>("browser_extension_status");
    } catch (e) {
      console.warn("[ext] status failed", e);
    }
  });

  async function exportExtension(browser: string) {
    if (busyBrowser) return;
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
        hint: status?.install_hint ?? "",
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
    <h5 class="section-title">Extensão de navegador</h5>
    <p class="muted">A extensão envia cookies e URLs do seu navegador para o OmniGet. Auto-update direto pelo app só funciona via lojas oficiais — enquanto não publicamos, use os botões abaixo para extrair a versão atual e (re)instalar manualmente.</p>
    <div class="card">
      {#each extStatuses as ext, i (ext.browser)}
        {#if i > 0}<div class="divider"></div>{/if}
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{browserLabel(ext.browser)}</span>
            <span class="setting-path">
              {#if ext.bundled_version}
                Versão empacotada: v{ext.bundled_version}
              {:else}
                {ext.install_hint}
              {/if}
            </span>
          </div>
          {#if ext.installable}
            <button
              class="button"
              disabled={busyBrowser === ext.browser}
              onclick={() => exportExtension(ext.browser)}
            >
              {busyBrowser === ext.browser ? "Extraindo..." : "Atualizar / Instalar"}
            </button>
          {:else if ext.store_url}
            <a class="button" href={ext.store_url} target="_blank" rel="noreferrer">Abrir loja</a>
          {:else}
            <span class="muted small">Indisponível</span>
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
                busy={installingDep === dep.name}
                onInstall={(variant) => onInstallDep(dep.name, variant)}
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
  <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeInstall(); }}>
    <div class="dialog" role="dialog" aria-modal="true">
      <header class="head">
        <h3>Instalar / Atualizar extensão — {browserLabel(installModal.browser)}</h3>
        <button class="close" onclick={closeInstall} aria-label="fechar">×</button>
      </header>
      <div class="body">
        <p>A extensão foi extraída para:</p>
        <code class="path">{installModal.path}</code>
        <button class="button" onclick={() => openFolder(installModal!.path)}>Abrir pasta</button>
        <h4>Como instalar / atualizar</h4>
        <p class="hint">{installModal.hint}</p>
        <p class="hint small">
          Se a extensão já está instalada, clique em "Recarregar" no <code>{installModal.browser === 'firefox' ? 'about:debugging' : 'chrome://extensions'}</code> apontando pra essa pasta — ou remova a antiga e adicione a nova.
        </p>
      </div>
      <footer class="foot">
        <button class="button" onclick={closeInstall}>Fechar</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .muted { color: rgba(255, 255, 255, 0.6); font-size: 12px; line-height: 1.4; margin: 0 0 8px; }
  .small { font-size: 11px; }
  .overlay {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6);
    display: grid; place-items: center; z-index: 999;
  }
  .dialog {
    background: var(--bg-card, #1a1a1a);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    width: min(560px, 90vw);
    max-height: 80vh; overflow: auto;
    display: flex; flex-direction: column;
  }
  .head {
    display: flex; justify-content: space-between; align-items: center;
    padding: 16px 20px; border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  }
  .head h3 { margin: 0; font-size: 16px; }
  .close {
    background: transparent; border: 0; color: rgba(255, 255, 255, 0.6);
    font-size: 22px; line-height: 1; cursor: pointer;
  }
  .body { padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  .body h4 { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .body p { margin: 0; font-size: 13px; line-height: 1.5; }
  .body .hint { color: rgba(255, 255, 255, 0.7); }
  .path {
    display: block; padding: 8px 10px; background: rgba(0, 0, 0, 0.4);
    border-radius: 6px; font-family: var(--font-mono, monospace); font-size: 11px;
    word-break: break-all;
  }
  .foot {
    padding: 12px 20px; border-top: 1px solid rgba(255, 255, 255, 0.05);
    display: flex; justify-content: flex-end;
  }
</style>
