<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { formatBytes } from "$lib/washmybelly/api";
  import {
    errorCode,
    filterProcesses,
    formatRate,
    machineCpu,
    sortProcesses,
    toggleSort,
    type ProcessList,
    type ProcessRow,
    type SortDir,
    type SortKey,
  } from "$lib/washmybelly/monitor";

  // Top processes, sortable and filterable, with "End" by exact PID.
  // Design after 21st.dev "Sortable Table" (ddoemonn): uppercase headers
  // that are buttons, an arrow only on the active column, hairline rows,
  // aria-sort and a polite live region; ported to Svelte 5 without motion.
  const LIMIT = 50;
  const REFRESH_MS = 3000;

  let list = $state<ProcessList | null>(null);
  let error = $state<string | null>(null);
  let query = $state("");
  let sort = $state<{ key: SortKey; dir: SortDir }>({ key: "cpu", dir: "desc" });
  let confirm = $state<ProcessRow | null>(null);
  let busy = $state(false);
  let notice = $state("");
  let timer: ReturnType<typeof setInterval> | null = null;
  let debounce: ReturnType<typeof setTimeout> | null = null;

  let rows = $derived(list ? sortProcesses(filterProcesses(list.processes, query), sort.key, sort.dir) : []);

  function serverSort(): "cpu" | "memory" | "disk" {
    return sort.key === "memory" || sort.key === "disk" ? sort.key : "cpu";
  }

  async function load() {
    if (typeof document !== "undefined" && document.hidden) return;
    try {
      list = await api.call<ProcessList>("wmb_processes", { sort: serverSort(), limit: LIMIT, ...(query.trim() ? { query: query.trim() } : {}) });
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    load();
    timer = setInterval(load, REFRESH_MS);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
    if (debounce) clearTimeout(debounce);
  });

  function onQuery() {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(load, 300);
  }

  function header(key: SortKey) {
    const before = serverSort();
    sort = toggleSort(sort, key);
    if (serverSort() !== before) load();
  }

  function ariaSort(key: SortKey): "ascending" | "descending" | "none" {
    return sort.key === key ? (sort.dir === "asc" ? "ascending" : "descending") : "none";
  }

  async function end(p: ProcessRow) {
    busy = true;
    try {
      await api.ui("monitor.kill", { pid: p.pid, started: p.started });
      notice = $t("washmybelly.monitor.processes.ended", { name: p.name }) as string;
      confirm = null;
      await load();
    } catch (e) {
      const code = errorCode(e);
      notice =
        code === "OTHER"
          ? String(e)
          : ($t(`washmybelly.monitor.processes.error_${code.toLowerCase()}`, { name: p.name }) as string);
      confirm = null;
    } finally {
      busy = false;
    }
  }

  const COLS: { key: SortKey; label: string; end?: boolean }[] = [
    { key: "name", label: "washmybelly.monitor.processes.col_name" },
    { key: "cpu", label: "washmybelly.monitor.processes.col_cpu", end: true },
    { key: "memory", label: "washmybelly.monitor.processes.col_memory", end: true },
    { key: "disk", label: "washmybelly.monitor.processes.col_disk", end: true },
    { key: "user", label: "washmybelly.monitor.processes.col_user" },
    { key: "pid", label: "washmybelly.monitor.processes.col_pid", end: true },
  ];
</script>

<section class="procs">
  <div class="bar">
    <h3>{$t("washmybelly.monitor.processes.title")}</h3>
    <input
      type="search"
      placeholder={$t("washmybelly.monitor.processes.filter") as string}
      aria-label={$t("washmybelly.monitor.processes.filter") as string}
      bind:value={query}
      oninput={onQuery}
    />
  </div>

  {#if confirm}
    <div class="confirm" role="alertdialog" aria-labelledby="wmb-end-title">
      <div>
        <strong id="wmb-end-title">{$t("washmybelly.monitor.processes.confirm_title", { name: confirm.name })}</strong>
        <p>{$t("washmybelly.monitor.processes.confirm_body")}</p>
      </div>
      <div class="btns">
        <button class="btn" onclick={() => (confirm = null)} disabled={busy}>{$t("washmybelly.monitor.processes.cancel")}</button>
        <button class="btn danger" onclick={() => confirm && end(confirm)} disabled={busy}>{$t("washmybelly.monitor.processes.end")}</button>
      </div>
    </div>
  {/if}

  {#if error && !list}
    <p class="error">{error}</p>
  {:else}
    <div class="table" role="table" aria-label={$t("washmybelly.monitor.processes.title") as string} aria-rowcount={rows.length + 1}>
      <div class="row head" role="row">
        {#each COLS as c (c.key)}
          <div role="columnheader" aria-sort={ariaSort(c.key)} class:end={c.end}>
            <button class="hbtn" class:on={sort.key === c.key} onclick={() => header(c.key)}>
              <span>{$t(c.label)}</span>
              <svg class="arrow" class:down={sort.dir === "desc"} class:show={sort.key === c.key} width="9" height="9" viewBox="0 0 10 10" aria-hidden="true">
                <path d="M5 8.6V1.6M5 1.6 2.2 4.4M5 1.6l2.8 2.8" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </div>
        {/each}
        <div role="columnheader"><span class="sr">{$t("washmybelly.monitor.processes.actions")}</span></div>
      </div>
      <div class="body" role="rowgroup">
        {#each rows as p (p.pid)}
          <div class="row" role="row" title={p.command}>
            <div role="cell" class="name">{p.name}</div>
            <div role="cell" class="num">{machineCpu(p.cpu_percent, list?.cpu_count ?? 1).toFixed(1)}%</div>
            <div role="cell" class="num">{formatBytes(p.memory_bytes)}</div>
            <div role="cell" class="num">{p.disk_read_per_s + p.disk_write_per_s > 0 ? formatRate(p.disk_read_per_s + p.disk_write_per_s) : "—"}</div>
            <div role="cell" class="dim">{p.user ?? "—"}</div>
            <div role="cell" class="num dim">{p.pid}</div>
            <div role="cell" class="act">
              {#if p.can_end}
                <button class="link" onclick={() => (confirm = p)} aria-label={$t("washmybelly.monitor.processes.end_named", { name: p.name }) as string}>
                  {$t("washmybelly.monitor.processes.end")}
                </button>
              {/if}
            </div>
          </div>
        {:else}
          <div class="row empty" role="row"><div role="cell">{list ? $t("washmybelly.monitor.processes.none") : $t("washmybelly.loading")}</div></div>
        {/each}
      </div>
    </div>
    {#if list}
      <p class="meta">{$t("washmybelly.monitor.processes.showing", { shown: rows.length, total: list.total })}</p>
    {/if}
  {/if}
  <div class="sr" role="status" aria-live="polite">{notice}</div>
  {#if notice}<p class="meta">{notice}</p>{/if}
</section>

<style>
  .procs { display: flex; flex-direction: column; gap: 8px; }
  .bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  input[type="search"] { font: inherit; font-size: 12px; padding: 4px 10px; border-radius: 7px; border: none; background: var(--surface-mut, var(--separator)); color: var(--text); width: 200px; }
  input[type="search"]:focus-visible { outline: 2px solid var(--accent); }
  .table { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); overflow: hidden; }
  .row { display: grid; grid-template-columns: minmax(0, 2.2fr) 64px 84px 88px minmax(0, 1fr) 64px 64px; align-items: center; gap: 8px; padding: 0 12px; min-height: 32px; font-size: 12.5px; border-top: var(--hairline, 1px) solid var(--separator); }
  .row.head { border-top: none; min-height: 30px; }
  .body { max-height: 420px; overflow-y: auto; overscroll-behavior: contain; }
  .body .row:first-child { border-top: var(--hairline, 1px) solid var(--separator); }
  .row.empty { display: flex; color: var(--text-dim); }
  .end, .num { text-align: right; }
  .end .hbtn { flex-direction: row-reverse; }
  .num { font-variant-numeric: tabular-nums; }
  .name { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hbtn { all: unset; cursor: pointer; display: flex; align-items: center; gap: 4px; width: 100%; font-size: 10.5px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-dim); border-radius: 5px; }
  .hbtn.on, .hbtn:hover { color: var(--text); }
  .hbtn:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .arrow { opacity: 0; transition: transform 0.15s ease, opacity 0.15s ease; }
  .arrow.show { opacity: 1; }
  .arrow.down { transform: rotate(180deg); }
  .act { text-align: right; }
  .link { all: unset; cursor: pointer; font-size: 12px; color: var(--danger, #ff453a); }
  .link:focus-visible { box-shadow: 0 0 0 2px var(--accent); border-radius: 4px; }
  .confirm { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 14px; border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: 0 6px 20px rgb(0 0 0 / 0.14), inset 0 0 0 var(--hairline, 1px) var(--separator); }
  .confirm p { margin: 2px 0 0; font-size: 12px; color: var(--text-dim); }
  .confirm strong { font-size: 13px; }
  .btns { display: flex; gap: 8px; }
  .btn { font: inherit; font-size: 12px; padding: 4px 12px; border-radius: 7px; border: none; cursor: pointer; background: var(--surface-mut, var(--separator)); color: var(--text); }
  .btn.danger { background: var(--danger, #ff453a); color: #fff; }
  .btn:focus-visible { outline: 2px solid var(--accent); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .meta { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  @media (prefers-reduced-motion: reduce) { .arrow { transition: none; } }
</style>
