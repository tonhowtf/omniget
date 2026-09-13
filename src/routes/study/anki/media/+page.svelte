<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type MediaEntry = {
    fname: string;
    csum: string | null;
    mtime: number;
    dirty: boolean;
  };

  type CheckReport = {
    total: number;
    used: number;
    unused: string[];
    missing: string[];
  };

  type TrashGroup = {
    id: number;
    fnames: string[];
    timestamp: number;
  };

  let entries = $state<MediaEntry[]>([]);
  let report = $state<CheckReport | null>(null);
  let recentTrash = $state<TrashGroup[]>([]);

  let loading = $state(true);
  let checking = $state(false);
  let trashing = $state(false);
  let restoring = $state<number | null>(null);
  let emptying = $state(false);
  let adding = $state(false);
  let confirmEmpty = $state(false);

  let error = $state("");
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let filter = $state("");
  let view = $state<"all" | "unused" | "missing">("all");
  let selected = $state<Set<string>>(new Set());

  let visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    let pool: string[] = [];
    if (view === "all") {
      pool = entries.map((e) => e.fname);
    } else if (view === "unused") {
      pool = report?.unused ?? [];
    } else {
      pool = report?.missing ?? [];
    }
    if (q) pool = pool.filter((f) => f.toLowerCase().includes(q));
    return pool;
  });

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const list = await pluginInvoke<MediaEntry[]>(
        "study",
        "study:anki:media:list",
      );
      entries = list ?? [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function runCheck() {
    checking = true;
    try {
      const r = await pluginInvoke<CheckReport>(
        "study",
        "study:anki:media:check",
      );
      report = r;
      showToast("ok", $t("study.anki.media.check_done"));
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      checking = false;
    }
  }

  async function pickAndAdd() {
    if (adding) return;
    adding = true;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({
        multiple: false,
        filters: [
          {
            name: $t("study.anki.media.dialog_media"),
            extensions: [
              "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp",
              "mp3", "wav", "ogg", "m4a", "flac",
              "mp4", "webm", "mov",
            ],
          },
        ],
      });
      if (typeof picked !== "string" || !picked) return;
      const r = await pluginInvoke<{ fname: string }>(
        "study",
        "study:anki:media:add",
        { sourcePath: picked },
      );
      showToast("ok", $t("study.anki.media.added", { name: r.fname }));
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      adding = false;
    }
  }

  function toggleSelect(fname: string) {
    if (selected.has(fname)) {
      selected.delete(fname);
    } else {
      selected.add(fname);
    }
    selected = new Set(selected);
  }

  function selectAllVisible() {
    selected = new Set([...selected, ...visible]);
  }

  function clearSelection() {
    selected = new Set();
  }

  async function trashSelected() {
    if (selected.size === 0 || trashing) return;
    const fnames = [...selected];
    trashing = true;
    try {
      const r = await pluginInvoke<{ moved: number }>(
        "study",
        "study:anki:media:trash",
        { fnames },
      );
      showToast(
        "ok",
        r.moved === 1 ? $t("study.anki.media.trashed_one") : $t("study.anki.media.trashed_many", { n: r.moved }),
      );
      if (r.moved > 0) {
        recentTrash = [
          { id: Date.now(), fnames, timestamp: Date.now() },
          ...recentTrash,
        ];
      }
      clearSelection();
      await load();
      if (report) await runCheck();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      trashing = false;
    }
  }

  async function trashUnused() {
    if (!report || report.unused.length === 0) return;
    const fnames = [...report.unused];
    trashing = true;
    try {
      const r = await pluginInvoke<{ moved: number }>(
        "study",
        "study:anki:media:trash",
        { fnames },
      );
      showToast(
        "ok",
        r.moved === 1 ? $t("study.anki.media.unused_trashed_one") : $t("study.anki.media.unused_trashed_many", { n: r.moved }),
      );
      if (r.moved > 0) {
        recentTrash = [
          { id: Date.now(), fnames, timestamp: Date.now() },
          ...recentTrash,
        ];
      }
      await load();
      await runCheck();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      trashing = false;
    }
  }

  async function restoreGroup(group: TrashGroup) {
    if (restoring !== null) return;
    restoring = group.id;
    try {
      const r = await pluginInvoke<{ restored: number }>(
        "study",
        "study:anki:media:restore",
        { fnames: group.fnames },
      );
      showToast(
        "ok",
        r.restored === 1 ? $t("study.anki.media.restored_one") : $t("study.anki.media.restored_many", { n: r.restored }),
      );
      recentTrash = recentTrash.filter((g) => g.id !== group.id);
      await load();
      if (report) await runCheck();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      restoring = null;
    }
  }

  async function emptyTrash() {
    confirmEmpty = false;
    emptying = true;
    try {
      const r = await pluginInvoke<{ removed: number }>(
        "study",
        "study:anki:media:empty_trash",
      );
      showToast(
        "ok",
        r.removed === 0 ? $t("study.anki.media.trash_was_empty") :
        r.removed === 1 ? $t("study.anki.media.deleted_forever_one") :
        $t("study.anki.media.deleted_forever_many", { n: r.removed }),
      );
      recentTrash = [];
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      emptying = false;
    }
  }

  function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return "—";
    const units = ["B", "KB", "MB", "GB"];
    let n = bytes;
    let i = 0;
    while (n >= 1024 && i < units.length - 1) {
      n /= 1024;
      i++;
    }
    return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
  }

  function fmtTime(secs: number): string {
    if (!secs || secs <= 0) return "—";
    return new Date(secs * 1000).toLocaleDateString();
  }

  function fmtRelative(ms: number): string {
    const diff = Date.now() - ms;
    const min = Math.floor(diff / 60000);
    if (min < 1) return $t("study.anki.media.relative_now");
    if (min < 60) return $t("study.anki.media.relative_min", { n: min });
    const h = Math.floor(min / 60);
    if (h < 24) return $t("study.anki.media.relative_h", { n: h });
    return new Date(ms).toLocaleDateString();
  }

  onMount(load);
</script>

<section class="study-page">
  <PageHero
    title={$t("study.anki.media.title")}
    subtitle={loading
      ? $t("study.anki.media.loading")
      : entries.length === 0
        ? $t("study.anki.media.subtitle_empty")
        : entries.length === 1
          ? $t("study.anki.media.one_file")
          : $t("study.anki.media.n_files", { n: entries.length })}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="toolbar">
    <div class="tabs" role="tablist" aria-label={$t("study.anki.media.filter_aria")}>
      <button
        type="button"
        class="tab"
        class:active={view === "all"}
        role="tab"
        aria-selected={view === "all"}
        onclick={() => (view = "all")}
      >
        {$t("study.anki.media.tab_all")}
        <span class="count">{entries.length}</span>
      </button>
      <button
        type="button"
        class="tab"
        class:active={view === "unused"}
        class:disabled={!report}
        role="tab"
        aria-selected={view === "unused"}
        onclick={() => (view = "unused")}
        disabled={!report}
        title={report ? "" : $t("study.anki.media.run_check_first")}
      >
        {$t("study.anki.media.tab_unused")}
        <span class="count">{report?.unused.length ?? "—"}</span>
      </button>
      <button
        type="button"
        class="tab"
        class:active={view === "missing"}
        class:disabled={!report}
        role="tab"
        aria-selected={view === "missing"}
        onclick={() => (view = "missing")}
        disabled={!report}
        title={report ? "" : "Rode \"Verificar\" primeiro"}
      >
        {$t("study.anki.media.tab_missing")}
        <span class="count">{report?.missing.length ?? "—"}</span>
      </button>
    </div>

    <div class="actions">
      <button
        type="button"
        class="btn ghost sm"
        onclick={runCheck}
        disabled={checking}
      >
        {checking ? $t("study.anki.media.verifying") : $t("study.anki.media.verify")}
      </button>
      <button
        type="button"
        class="btn primary sm"
        onclick={pickAndAdd}
        disabled={adding}
      >
        {adding ? $t("study.anki.media.adding") : $t("study.anki.media.add_file")}
      </button>
    </div>
  </div>

  {#if report && view === "unused" && report.unused.length > 0}
    <div class="unused-banner">
      <span>{report.unused.length === 1 ? $t("study.anki.media.unused_banner_one", { n: 1 }) : $t("study.anki.media.unused_banner_many", { n: report.unused.length })}</span>
      <button
        type="button"
        class="btn ghost sm"
        onclick={trashUnused}
        disabled={trashing}
      >
        {trashing ? $t("study.anki.media.moving") : $t("study.anki.media.trash_all")}
      </button>
    </div>
  {/if}

  {#if view !== "missing"}
    <div class="search-row">
      <input
        type="search"
        class="search"
        placeholder={$t("study.anki.media.filter_files")}
        bind:value={filter}
      />
      {#if selected.size > 0}
        <span class="sel-count">
          {$t("study.anki.media.selected_count", { n: selected.size })}
        </span>
        <button
          type="button"
          class="btn ghost sm"
          onclick={clearSelection}
        >
          {$t("study.common.clear")}
        </button>
        <button
          type="button"
          class="btn ghost sm danger"
          onclick={trashSelected}
          disabled={trashing}
        >
          {trashing ? $t("study.anki.media.moving") : $t("study.anki.media.trash_selected")}
        </button>
      {:else if visible.length > 0}
        <button
          type="button"
          class="btn ghost sm"
          onclick={selectAllVisible}
        >
          {$t("study.anki.media.select_visible")}
        </button>
      {/if}
    </div>
  {/if}

  {#if loading}
    <div class="state">{$t("study.anki.media.loading_files")}</div>
  {:else if error}
    <div class="state err">{error}</div>
    <button class="btn ghost" onclick={load}>{$t("study.anki.settings.try_again")}</button>
  {:else if visible.length === 0}
    <div class="empty">
      {#if view === "missing"}
        <p>{$t("study.anki.media.no_missing")}</p>
        {#if !report}
          <p class="hint">{$t("study.anki.media.run_check_to_start")}</p>
        {/if}
      {:else if view === "unused"}
        {#if !report}
          <p>{$t("study.anki.media.not_ran_yet")}</p>
          <p class="hint">{$t("study.anki.media.check_to_find_unused")}</p>
        {:else}
          <p>{$t("study.anki.media.nothing_unused")}</p>
        {/if}
      {:else}
        <p>{$t("study.anki.media.no_media")}</p>
        <p class="hint">{$t("study.anki.media.add_hint")}</p>
      {/if}
    </div>
  {:else if view === "missing"}
    <ul class="file-list">
      {#each visible as fname (fname)}
        <li class="file-row missing-row">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 3l10 18H2z M12 10v5 M12 18v.5" />
          </svg>
          <span class="fname">{fname}</span>
          <span class="muted">{$t("study.anki.media.in_db_not_disk")}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <ul class="file-list">
      {#each visible as fname (fname)}
        {@const entry = entries.find((e) => e.fname === fname)}
        <li class="file-row">
          <input
            type="checkbox"
            class="row-check"
            checked={selected.has(fname)}
            onchange={() => toggleSelect(fname)}
            aria-label={$t("study.anki.media.select_one", { name: fname })}
          />
          <span class="fname">{fname}</span>
          {#if entry}
            <span class="muted">{fmtTime(entry.mtime)}</span>
            {#if entry.csum}
              <span class="csum">{entry.csum.slice(0, 8)}</span>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if recentTrash.length > 0}
    <section class="trash-section">
      <header class="trash-head">
        <h3>{$t("study.anki.media.trash_session")}</h3>
        <button
          type="button"
          class="btn ghost sm danger"
          onclick={() => (confirmEmpty = true)}
          disabled={emptying}
        >
          {emptying ? $t("study.anki.media.emptying") : $t("study.anki.media.empty_trash")}
        </button>
      </header>
      <p class="trash-lede">
        {$t("study.anki.media.trash_lede")}
      </p>
      <ul class="trash-list">
        {#each recentTrash as group (group.id)}
          <li class="trash-row">
            <div class="trash-info">
              <span class="trash-count">
                {group.fnames.length === 1
                  ? $t("study.anki.media.one_file")
                  : $t("study.anki.media.n_files", { n: group.fnames.length })}
              </span>
              <span class="trash-when">{fmtRelative(group.timestamp)}</span>
              <details class="trash-detail">
                <summary>{$t("study.anki.media.view_list")}</summary>
                <ul class="trash-fnames">
                  {#each group.fnames.slice(0, 20) as f (f)}
                    <li>{f}</li>
                  {/each}
                  {#if group.fnames.length > 20}
                    <li class="muted">{$t("study.anki.media.more_n", { n: group.fnames.length - 20 })}</li>
                  {/if}
                </ul>
              </details>
            </div>
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => restoreGroup(group)}
              disabled={restoring !== null}
            >
              {restoring === group.id ? $t("study.anki.media.restoring") : $t("study.anki.settings.restore")}
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</section>

{#if confirmEmpty}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={() => (confirmEmpty = false)}
    onkeydown={(e) => { if (e.key === "Escape") confirmEmpty = false; }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="empty-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); confirmEmpty = false; } }}
    >
      <h3 id="empty-title">{$t("study.anki.media.empty_confirm_title")}</h3>
      <p class="modal-body">
        {$t("study.anki.media.empty_confirm_body")}
      </p>
      <footer class="modal-foot">
        <button
          type="button"
          class="btn ghost"
          onclick={() => (confirmEmpty = false)}
        >
          {$t("study.common.cancel")}
        </button>
        <button
          type="button"
          class="btn primary danger"
          onclick={emptyTrash}
        >
          {$t("study.anki.media.empty")}
        </button>
      </footer>
    </div>
  </div>
{/if}

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
  .tab:hover:not(.active):not(.disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .tab.disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .tab .count {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--tertiary);
  }
  .tab.active .count {
    color: var(--accent);
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .unused-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
    background: color-mix(in oklab, var(--warning, var(--accent)) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--warning, var(--accent)) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-size: 13px;
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .search {
    flex: 1;
    padding: 7px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .search:focus {
    outline: none;
    border-color: var(--accent);
  }
  .sel-count {
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
  }

  .state {
    padding: 24px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }

  .empty {
    padding: 48px 16px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .empty .hint {
    margin: 4px 0 0;
    color: var(--tertiary);
    font-size: 12px;
  }

  .file-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .file-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-radius: var(--border-radius);
  }
  .file-row:hover {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .row-check {
    flex-shrink: 0;
    accent-color: var(--accent);
  }
  .fname {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .muted {
    color: var(--tertiary);
    font-size: 11px;
    flex-shrink: 0;
  }
  .csum {
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
    flex-shrink: 0;
  }

  .missing-row {
    color: var(--error, var(--accent));
  }

  .trash-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: calc(var(--padding) * 1.25);
    border-top: none;
  }
  .trash-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .trash-head h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
  }
  .trash-lede {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .trash-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .trash-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: none;
    border-radius: var(--border-radius);
  }
  .trash-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12px;
  }
  .trash-count {
    color: var(--text);
    font-weight: 500;
  }
  .trash-when {
    color: var(--tertiary);
    font-size: 11px;
  }
  .trash-detail summary {
    cursor: pointer;
    color: var(--accent);
    font-size: 11px;
    margin-top: 4px;
  }
  .trash-fnames {
    list-style: none;
    margin: 6px 0 0;
    padding: 6px 10px;
    background: var(--bg);
    border-radius: var(--border-radius);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
    max-height: 200px;
    overflow-y: auto;
  }
  .trash-fnames li {
    padding: 1px 0;
  }

  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover, var(--accent));
  }
  .btn.primary.danger {
    background: var(--error, var(--accent));
    color: var(--on-error, var(--on-cta, white));
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.ghost.danger {
    color: var(--error, var(--accent));
    border-color: color-mix(in oklab, var(--error, var(--accent)) 35%, var(--input-border));
  }
  .btn.ghost.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error, var(--accent)) 10%, transparent);
  }
  .btn.sm {
    padding: 5px 10px;
    font-size: 12px;
  }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 20%, transparent);
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
    max-width: 420px;
    width: 100%;
    box-shadow: 0 20px 50px color-mix(in oklab, black 30%, transparent);
  }
  .modal h3 {
    margin: 0 0 12px;
    font-size: 15px;
    font-weight: 600;
  }
  .modal-body {
    margin: 0 0 16px;
    color: var(--secondary);
    font-size: 13px;
    line-height: 1.55;
  }
  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    border-top: none;
    padding-top: 12px;
  }
</style>
