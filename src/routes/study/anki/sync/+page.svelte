<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    ankiOpen,
    ankiSyncProviderGet,
    ankiSyncProviderSave,
    ankiSyncProviderTest,
    ankiSyncRun,
    ankiPending,
    type ProviderConfig,
    type ProviderInfo,
    type SyncOutcome,
    type AnkiPendingCounts,
  } from "$lib/anki-bridge";
  import AnkiCard from "$lib/study-components/anki/AnkiCard.svelte";
  import AnkiButton from "$lib/study-components/anki/AnkiButton.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Kind = "local_folder" | "webdav" | "colpkg";

  let info = $state<ProviderInfo | null>(null);
  let pending = $state<AnkiPendingCounts | null>(null);
  let lastOutcome = $state<SyncOutcome | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state("");

  let chosen = $state<Kind>("local_folder");
  let folderPath = $state("");
  let webdavUrl = $state("");
  let webdavUser = $state("");
  let webdavPass = $state("");
  let colpkgDir = $state("");

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = "";
    try {
      await ankiOpen();
      const [i, p] = await Promise.all([ankiSyncProviderGet(), ankiPending()]);
      info = i;
      pending = p;
      hydrateForm(i.provider);
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? $t("study.common.error"));
    } finally {
      loading = false;
    }
  }

  function hydrateForm(p: ProviderConfig) {
    if (p.kind === "local_folder") {
      chosen = "local_folder";
      folderPath = p.path;
    } else if (p.kind === "webdav") {
      chosen = "webdav";
      webdavUrl = p.url;
      webdavUser = p.username;
      webdavPass = p.password;
    } else if (p.kind === "colpkg") {
      chosen = "colpkg";
      colpkgDir = p.dir;
    }
  }

  function buildConfig(): ProviderConfig {
    if (chosen === "local_folder") return { kind: "local_folder", path: folderPath.trim() };
    if (chosen === "webdav") {
      return {
        kind: "webdav",
        url: webdavUrl.trim(),
        username: webdavUser.trim(),
        password: webdavPass,
      };
    }
    return { kind: "colpkg", dir: colpkgDir.trim() };
  }

  function isFormValid(): boolean {
    if (chosen === "local_folder") return folderPath.trim().length > 0;
    if (chosen === "webdav") {
      return webdavUrl.trim().length > 0 && webdavUser.trim().length > 0 && webdavPass.length > 0;
    }
    return colpkgDir.trim().length > 0;
  }

  async function save() {
    if (busy || !isFormValid()) return;
    busy = true;
    try {
      const cfg = buildConfig();
      await ankiSyncProviderSave(cfg);
      showToast("info", "Provedor salvo");
      await load();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Erro"));
    } finally {
      busy = false;
    }
  }

  async function test() {
    if (busy || !isFormValid()) return;
    busy = true;
    try {
      await ankiSyncProviderTest(buildConfig());
      showToast("info", $t("study.anki.sync.connection_ok"));
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : (e?.message ?? "Falhou"));
    } finally {
      busy = false;
    }
  }

  async function run() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const outcome = await ankiSyncRun();
      lastOutcome = outcome;
      showToast(outcome.action === "no_provider" ? "error" : "info", outcome.message);
      await load();
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? $t("study.common.error"));
    } finally {
      busy = false;
    }
  }

  function fmtDate(secs: number): string {
    if (!secs) return $t("study.tg_sync.never");
    return new Date(secs * 1000).toLocaleString();
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
</script>

<div class="sync-page">
  <header class="page-head">
    <h1>Sincronização</h1>
    <p class="muted">
      Sync por <strong>arquivos</strong>: collection.anki2 + media via WebDAV, pasta local ou backup .colpkg.
      Não usa servidor AnkiWeb.
    </p>
  </header>

  {#if loading}
    <div class="loader"><div class="ring"></div></div>
  {:else if error}
    <AnkiCard variant="ghost" padding="m">
      <p class="error">{error}</p>
      <AnkiButton onclick={load} variant="primary">Tentar novamente</AnkiButton>
    </AnkiCard>
  {:else}
    <section class="status-row">
      <AnkiCard padding="m" variant={info && info.kind !== "none" ? "highlight" : "default"}>
        <div class="status-grid">
          <div>
            <span class="kpi-tag">Provedor ativo</span>
            <strong class="kpi">{info?.display ?? "—"}</strong>
          </div>
          <div>
            <span class="kpi-tag">Último sync</span>
            <strong class="kpi">{info ? fmtDate(info.last_sync_secs) : "—"}</strong>
          </div>
          <div>
            <span class="kpi-tag">Mudanças locais</span>
            <strong class="kpi">{pending?.total ?? 0}</strong>
          </div>
        </div>
        <div class="run-row">
          <AnkiButton
            variant="primary"
            size="l"
            disabled={busy || !info || info.kind === "none"}
            onclick={run}
          >
            {busy ? $t("study.anki.syncing") : `☁️ ${$t("study.anki.sync_now")}`}
          </AnkiButton>
        </div>
      </AnkiCard>
    </section>

    {#if lastOutcome}
      <AnkiCard padding="m" variant="ghost">
        <h3 class="card-h">Resultado</h3>
        <div class="outcome">
          <span class="ank-pill ank-pill--info">{lastOutcome.action}</span>
          <p class="outcome-msg">{lastOutcome.message}</p>
          <p class="outcome-bytes">
            ↑ {fmtBytes(lastOutcome.bytes_uploaded)} · ↓ {fmtBytes(lastOutcome.bytes_downloaded)}
          </p>
        </div>
      </AnkiCard>
    {/if}

    <section class="provider-section">
      <h2>Configurar provedor</h2>
      <div class="provider-tabs" role="tablist">
        <button class="ptab" class:active={chosen === "local_folder"} onclick={() => (chosen = "local_folder")}>
          📁 Pasta local
        </button>
        <button class="ptab" class:active={chosen === "webdav"} onclick={() => (chosen = "webdav")}>
          ☁️ WebDAV
        </button>
        <button class="ptab" class:active={chosen === "colpkg"} onclick={() => (chosen = "colpkg")}>
          💾 Backup .colpkg
        </button>
      </div>

      <AnkiCard padding="m">
        {#if chosen === "local_folder"}
          <p class="hint">
            Use uma pasta sincronizada por Dropbox/Google Drive/iCloud Drive. O OmniGet escreve
            <code>collection.anki2</code> + <code>media.zip</code> + <code>manifest.json</code>.
          </p>
          <label class="field">
            <span>Caminho da pasta</span>
            <input class="input" type="text" placeholder="C:\Users\you\Dropbox\anki" bind:value={folderPath} />
          </label>
        {:else if chosen === "webdav"}
          <p class="hint">
            Compatível com Nextcloud, ownCloud, Synology, Apache mod_dav. Use HTTPS sempre que possível.
          </p>
          <label class="field">
            <span>URL</span>
            <input class="input" type="url" placeholder="https://nuvem.exemplo.com/remote.php/dav/files/me/anki/" bind:value={webdavUrl} />
          </label>
          <label class="field">
            <span>Usuário</span>
            <input class="input" type="text" autocomplete="username" bind:value={webdavUser} />
          </label>
          <label class="field">
            <span>Senha (de app, recomendado)</span>
            <input class="input" type="password" autocomplete="new-password" bind:value={webdavPass} />
          </label>
        {:else}
          <p class="hint">
            Backup manual em <code>.colpkg</code>. Cada sync gera um arquivo novo timestamped.
            Você pode importar de volta via Importar.
          </p>
          <label class="field">
            <span>Pasta de destino</span>
            <input class="input" type="text" placeholder="C:\Users\you\Documents\anki-backups" bind:value={colpkgDir} />
          </label>
        {/if}

        <div class="actions">
          <AnkiButton variant="outline" onclick={test} disabled={busy || !isFormValid()}>
            Testar conexão
          </AnkiButton>
          <AnkiButton variant="primary" onclick={save} disabled={busy || !isFormValid()}>
            Salvar provedor
          </AnkiButton>
        </div>
      </AnkiCard>
    </section>
  {/if}
</div>

<style>
  .sync-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 880px;
  }

  .page-head h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--text-sm);
    margin: var(--space-2) 0 0;
  }

  .loader {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-7);
  }
  .ring {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 3px solid var(--surface-mut);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .error { color: var(--error); margin: 0 0 var(--space-3); }

  .status-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--space-4);
  }
  .kpi-tag {
    display: block;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }
  .kpi {
    font-size: var(--text-md);
    font-weight: 600;
  }
  .run-row {
    margin-top: var(--space-4);
    display: flex;
  }

  .provider-section h2 {
    margin: 0 0 var(--space-3);
    font-size: 16px;
    font-weight: 600;
  }
  .provider-tabs {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
    flex-wrap: wrap;
  }
  .ptab {
    padding: var(--space-2) var(--space-4);
    background: var(--surface-hi);
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-family: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .ptab:hover {
    background: var(--surface-mut);
    color: var(--text);
  }
  .ptab.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }

  .hint {
    margin: 0 0 var(--space-3);
    color: var(--text-muted);
    font-size: var(--text-xs);
    line-height: 1.5;
  }
  .hint code {
    font-family: monospace;
    background: var(--surface-mut);
    padding: 1px 5px;
    border-radius: 4px;
    font-size: 0.9em;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: var(--space-3);
  }
  .field span {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
    font-weight: 600;
  }
  .input {
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-sm);
  }
  .input:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
    margin-top: var(--space-3);
  }

  .card-h { margin: 0 0 var(--space-2); font-size: var(--text-md); font-weight: 600; }
  .outcome { display: flex; flex-direction: column; gap: var(--space-2); }
  .outcome-msg { margin: 0; font-size: var(--text-sm); }
  .outcome-bytes { margin: 0; font-size: var(--text-xs); color: var(--text-muted); }
</style>
