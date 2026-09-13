<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type DownloadStatus =
    | "queued"
    | "downloading"
    | "complete"
    | "error"
    | "cancelled";

  type Download = {
    id: number;
    source_id: string;
    book_ref: string;
    title: string | null;
    author: string | null;
    status: DownloadStatus;
    progress: number;
    bytes_downloaded: number;
    total_bytes: number | null;
    error: string | null;
    created_secs: number;
    updated_secs: number;
  };

  type TorrentMirror = {
    name: string;
    magnet: string;
    size_label: string | null;
    seeders: number | null;
  };

  let downloads = $state<Download[]>([]);
  let loading = $state(true);
  let error = $state("");
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let filter = $state<"all" | "active" | "finished" | "errored">("all");
  let acting = $state<number | null>(null);
  let clearingFinished = $state(false);
  let confirmClearOpen = $state(false);

  let torrentsTarget = $state<{ sourceId: string; bookRef: string; title: string } | null>(null);
  let torrentsLoading = $state(false);
  let torrents = $state<TorrentMirror[]>([]);

  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    try {
      const filters: Record<string, unknown> = {};
      if (filter === "active") filters.status = ["queued", "downloading"];
      else if (filter === "finished") filters.status = ["complete"];
      else if (filter === "errored") filters.status = ["error", "cancelled"];

      const list = await pluginInvoke<Download[]>(
        "study",
        "study:read:downloads:list",
        { filters: Object.keys(filters).length > 0 ? filters : null },
      );
      downloads = list ?? [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function startPolling() {
    stopPolling();
    pollTimer = setInterval(() => {
      const hasActive = downloads.some(
        (d) => d.status === "queued" || d.status === "downloading",
      );
      if (hasActive) void load();
    }, 2000);
  }

  function stopPolling() {
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  async function retry(d: Download) {
    acting = d.id;
    try {
      await pluginInvoke("study", "study:read:downloads:retry", { id: d.id });
      showToast("ok", "Retry agendado");
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      acting = null;
    }
  }

  async function cancel(d: Download) {
    acting = d.id;
    try {
      await pluginInvoke("study", "study:read:downloads:cancel", { id: d.id });
      showToast("ok", "Cancelado");
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      acting = null;
    }
  }

  async function clearFinished() {
    confirmClearOpen = false;
    clearingFinished = true;
    try {
      const r = await pluginInvoke<{ removed: number }>(
        "study",
        "study:read:downloads:clear_finished",
      );
      showToast(
        "ok",
        r.removed === 0
          ? "Nada pra limpar"
          : r.removed === 1
            ? "1 entrada removida"
            : `${r.removed} entradas removidas`,
      );
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      clearingFinished = false;
    }
  }

  async function showTorrents(d: Download) {
    torrentsTarget = {
      sourceId: d.source_id,
      bookRef: d.book_ref,
      title: d.title ?? d.book_ref,
    };
    torrents = [];
    torrentsLoading = true;
    try {
      const list = await pluginInvoke<TorrentMirror[]>(
        "study",
        "study:read:downloads:list_torrents",
        { sourceId: d.source_id, bookRef: d.book_ref },
      );
      torrents = list ?? [];
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      torrentsLoading = false;
    }
  }

  function copyMagnet(m: TorrentMirror) {
    if (navigator.clipboard) {
      navigator.clipboard.writeText(m.magnet);
      showToast("ok", "Magnet copiado");
    }
  }

  function fmtBytes(n: number | null): string {
    if (!n || n <= 0) return "—";
    const units = ["B", "KB", "MB", "GB"];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
  }

  function fmtPct(n: number): string {
    return `${Math.max(0, Math.min(100, Math.round(n))).toString()}%`;
  }

  function statusLabel(s: DownloadStatus): string {
    return {
      queued: "fila",
      downloading: "baixando",
      complete: "ok",
      error: "erro",
      cancelled: "cancelado",
    }[s];
  }

  let counts = $derived.by(() => {
    let active = 0,
      finished = 0,
      errored = 0;
    for (const d of downloads) {
      if (d.status === "queued" || d.status === "downloading") active++;
      else if (d.status === "complete") finished++;
      else errored++;
    }
    return { active, finished, errored, total: downloads.length };
  });

  $effect(() => {
    void filter;
    void load();
  });

  onMount(() => {
    void load();
    startPolling();
  });
  onDestroy(stopPolling);
</script>

<section class="study-page">
  <PageHero
    title="Downloads de livros"
    subtitle={loading
      ? "Carregando…"
      : counts.active > 0
        ? counts.active === 1
          ? "1 ativo"
          : `${counts.active} ativos`
        : "Nenhum download ativo"}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="toolbar">
    <div class="tabs" role="tablist">
      <button
        type="button"
        class="tab"
        class:active={filter === "all"}
        onclick={() => (filter = "all")}
      >
        Todos <span class="count">{counts.total}</span>
      </button>
      <button
        type="button"
        class="tab"
        class:active={filter === "active"}
        onclick={() => (filter = "active")}
      >
        Ativos <span class="count">{counts.active}</span>
      </button>
      <button
        type="button"
        class="tab"
        class:active={filter === "finished"}
        onclick={() => (filter = "finished")}
      >
        Concluídos <span class="count">{counts.finished}</span>
      </button>
      <button
        type="button"
        class="tab"
        class:active={filter === "errored"}
        onclick={() => (filter = "errored")}
      >
        Falhas <span class="count">{counts.errored}</span>
      </button>
    </div>
    <button
      type="button"
      class="btn ghost sm"
      onclick={() => (confirmClearOpen = true)}
      disabled={clearingFinished || (counts.finished === 0 && counts.errored === 0)}
    >
      {clearingFinished ? $t("study.read.downloads.clearing") : $t("study.read.downloads.clear_finished")}
    </button>
  </div>

  {#if error}
    <div class="state err">{error}</div>
  {:else if loading}
    <div class="state">Carregando…</div>
  {:else if downloads.length === 0}
    <div class="empty">
      <p>Nenhum download {filter !== "all" ? "neste filtro" : "ainda"}.</p>
      {#if filter === "all"}
        <p class="hint">
          Use <a href="/study/read/discover">Descobrir</a> pra encontrar livros.
        </p>
      {/if}
    </div>
  {:else}
    <ul class="dl-list">
      {#each downloads as d (d.id)}
        <li class="dl-row" data-status={d.status}>
          <div class="dl-info">
            <div class="dl-title">
              {d.title ?? d.book_ref}
              <span class="dl-status status-{d.status}">{statusLabel(d.status)}</span>
            </div>
            {#if d.author}
              <div class="dl-author">{d.author}</div>
            {/if}
            <div class="dl-meta">
              {d.source_id}
              {#if d.total_bytes != null}
                · {fmtBytes(d.bytes_downloaded)} / {fmtBytes(d.total_bytes)}
              {:else if d.bytes_downloaded > 0}
                · {fmtBytes(d.bytes_downloaded)}
              {/if}
            </div>
            {#if d.status === "downloading" || d.status === "queued"}
              <div class="progress">
                <div class="progress-bar" style:width={fmtPct(d.progress * 100)}></div>
                <span class="progress-text">{fmtPct(d.progress * 100)}</span>
              </div>
            {/if}
            {#if d.error}
              <div class="dl-error" title={d.error}>{d.error}</div>
            {/if}
          </div>
          <div class="dl-actions">
            {#if d.status === "error" || d.status === "cancelled"}
              <button
                type="button"
                class="btn ghost sm"
                onclick={() => retry(d)}
                disabled={acting === d.id}
              >
                {acting === d.id ? "…" : "Retry"}
              </button>
            {/if}
            {#if d.status === "queued" || d.status === "downloading"}
              <button
                type="button"
                class="btn ghost sm"
                onclick={() => cancel(d)}
                disabled={acting === d.id}
              >
                {acting === d.id ? "…" : "Cancelar"}
              </button>
            {/if}
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => showTorrents(d)}
              title={$t("study.read.downloads.list_mirrors")}
            >
              Mirrors…
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if torrentsTarget}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) torrentsTarget = null; }}
  >
    <div class="modal modal-wide" role="dialog" aria-modal="true">
      <h3>Mirrors torrent</h3>
      <p class="modal-hint">{torrentsTarget.title}</p>
      {#if torrentsLoading}
        <p class="muted small">Buscando mirrors…</p>
      {:else if torrents.length === 0}
        <p class="muted small">Nenhum mirror torrent encontrado.</p>
      {:else}
        <ul class="torrent-list">
          {#each torrents as t (t.magnet)}
            <li class="torrent-row">
              <div class="torrent-info">
                <span class="torrent-name">{t.name}</span>
                <span class="torrent-meta">
                  {t.size_label ?? "—"}
                  {#if t.seeders != null}
                    · {t.seeders} seeders
                  {/if}
                </span>
              </div>
              <button
                type="button"
                class="btn ghost sm"
                onclick={() => copyMagnet(t)}
              >
                Copiar magnet
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="modal-foot">
        <button
          type="button"
          class="btn primary"
          onclick={() => (torrentsTarget = null)}
        >
          Fechar
        </button>
      </div>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmClearOpen}
  title={$t("study.read.downloads.clear_finished_title")}
  message={$t("study.read.downloads.clear_finished_body")}
  confirmLabel="Limpar"
  variant="danger"
  onConfirm={clearFinished}
/>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 900px;
    margin-inline: auto;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 0;
    background: transparent;
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: background 100ms ease;
  }
  .tab.active {
    background: var(--surface);
    color: var(--text);
  }
  .tab:hover:not(.active) {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .tab .count {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--tertiary);
  }
  .tab.active .count {
    color: var(--accent);
  }

  .state, .empty {
    padding: 32px 16px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .empty .hint {
    margin: 8px 0 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .empty a {
    color: var(--accent);
  }
  .state.err { color: var(--error, var(--accent)); }

  .dl-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .dl-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
  }
  .dl-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dl-title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: 14px;
    font-weight: 500;
  }
  .dl-status {
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .status-queued, .status-downloading {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .status-complete {
    background: color-mix(in oklab, var(--success, var(--accent)) 14%, transparent);
    color: var(--success, var(--accent));
  }
  .status-error, .status-cancelled {
    background: color-mix(in oklab, var(--error, var(--accent)) 14%, transparent);
    color: var(--error, var(--accent));
  }

  .dl-author {
    font-size: 12px;
    color: var(--secondary);
  }
  .dl-meta {
    color: var(--tertiary);
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .dl-error {
    margin-top: 4px;
    padding: 4px 8px;
    background: color-mix(in oklab, var(--error, var(--accent)) 8%, transparent);
    border-left: 2px solid var(--error, var(--accent));
    color: var(--error, var(--accent));
    font-size: 11px;
    border-radius: 3px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 480px;
  }

  .progress {
    position: relative;
    margin-top: 4px;
    height: 4px;
    background: var(--bg);
    border-radius: 2px;
    overflow: hidden;
  }
  .progress-bar {
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease;
  }
  .progress-text {
    position: absolute;
    right: 0;
    top: -16px;
    font-size: 10px;
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .dl-actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-shrink: 0;
  }

  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.sm { padding: 4px 10px; }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
    z-index: 100;
  }
  .toast.err {
    background: color-mix(in oklab, var(--error, var(--accent)) 18%, var(--surface));
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.55));
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    padding: 16px;
  }
  .modal {
    background: var(--popup-bg, var(--surface));
    border: none;
    border-radius: var(--border-radius);
    padding: 20px;
    max-width: 480px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modal-wide { max-width: 640px; }
  .modal h3 { margin: 0; font-size: 15px; font-weight: 600; }
  .modal-hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    border-top: none;
    padding-top: 12px;
  }

  .torrent-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 50vh;
    overflow-y: auto;
  }
  .torrent-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
  }
  .torrent-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .torrent-name {
    font-size: 13px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .torrent-meta {
    color: var(--tertiary);
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .muted { color: var(--tertiary); }
  .small { font-size: 12px; }
</style>
