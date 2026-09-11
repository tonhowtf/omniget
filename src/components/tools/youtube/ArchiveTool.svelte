<script lang="ts">
  /** Arquivar canal, playlist ou Watch Later, com estado por item e retomada. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Item = { id: string; title: string; url: string; duration_seconds: number; status: "pending" | "ok" | "failed" | "skipped"; error: string | null; file: string | null; attempts: number };
  type Result = { dest: string; state_path: string; title: string; source_url: string; total: number; done: number; failed: number; skipped: number; pending: number; items: Item[]; used_session: boolean; cancelled: boolean };

  const QUALITIES = ["best", "1080", "720", "480", "audio"];
  const STATUS_KEY: Record<Item["status"], string> = { pending: "tools.ytarchive.st_pending", ok: "tools.ytarchive.st_ok", failed: "tools.ytarchive.st_failed", skipped: "tools.ytarchive.st_skipped" };

  let url = $state("");
  let dest = $state("");
  let quality = $state("best");
  let writeSubs = $state(false);
  let writeThumb = $state(false);
  let writeInfo = $state(false);
  let maxItems = $state(0);
  let retryFailed = $state(false);
  let busy = $state(false);
  let scanning = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let itemPct = $state<number | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const shown = $derived(result ? result.items.slice(0, 200) : []);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id !== "yt-archive") return;
      if (p.stage === "item") itemPct = pct(p);
      else {
        progress = p;
        itemPct = null;
      }
    });
  });
  onDestroy(() => unlisten?.());

  function opts(refresh: boolean) {
    return { url, dest, quality, write_subs: writeSubs, write_thumbnail: writeThumb, write_info_json: writeInfo, max_items: Number(maxItems) || 0, refresh, retry_failed: retryFailed, account_slug: null };
  }

  async function chooseDest() {
    const d = await pickDir();
    if (!d) return;
    dest = d;
    await loadState();
  }

  async function loadState() {
    if (!dest) return;
    try {
      result = await invoke<Result>("tool_yt_archive_state", { dest });
    } catch {
      result = null;
    }
  }

  async function scan() {
    if (!url.trim() || !dest || busy || scanning) return;
    scanning = true;
    try {
      result = await invoke<Result>("tool_yt_archive_scan", { opts: opts(true) });
      showToast("success", `${result.total} ${$t("tools.ytarchive.items")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      scanning = false;
    }
  }

  async function run() {
    if (!dest || busy) return;
    if (!url.trim() && !result) return;
    busy = true;
    progress = null;
    try {
      result = await invoke<Result>("tool_yt_archive_run", { opts: opts(false) });
      if (result.cancelled) showToast("info", $t("tools.ytarchive.stopped") as string);
      else showToast("success", `${result.done}/${result.total}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
      itemPct = null;
    }
  }

  async function stop() {
    await invoke("tool_yt_archive_cancel");
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.ytarchive.url")} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytarchive.dest")}</div><div class="group-row-sub mono">{dest || "—"}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={chooseDest}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytarchive.quality")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={quality}>
            {#each QUALITIES as q (q)}<option value={q}>{$t(`tools.ytarchive.q_${q}`)}</option>{/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytarchive.extras")}</div><div class="group-row-sub">{$t("tools.ytarchive.extras_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="chk"><input type="checkbox" bind:checked={writeSubs} /> {$t("tools.ytarchive.subs")}</label>
          <label class="chk"><input type="checkbox" bind:checked={writeThumb} /> {$t("tools.ytarchive.thumb")}</label>
          <label class="chk"><input type="checkbox" bind:checked={writeInfo} /> {$t("tools.ytarchive.info")}</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytarchive.max_items")}</div><div class="group-row-sub">{$t("tools.ytarchive.max_items_sub")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="5" bind:value={maxItems} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ytarchive.retry")}</div><div class="group-row-sub">{$t("tools.ytarchive.retry_sub")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={retryFailed} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{itemPct ?? pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if busy}<button class="btn btn-ghost btn-sm" type="button" onclick={stop}>{$t("tools.ytarchive.stop")}</button>{/if}
          <button class="btn btn-secondary" type="button" disabled={busy || scanning || !url.trim() || !dest} onclick={scan}>{scanning ? $t("tools.common.working") : $t("tools.ytarchive.scan")}</button>
          <button class="btn btn-primary" type="button" disabled={busy || !dest} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ytarchive.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{result.title || result.source_url}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.done}/{result.total} {$t("tools.ytarchive.items")}</div>
            <div class="group-row-sub">
              {result.pending} {$t("tools.ytarchive.st_pending")} · {result.failed} {$t("tools.ytarchive.st_failed")} · {result.skipped} {$t("tools.ytarchive.st_skipped")}
              · {result.used_session ? $t("tools.ytarchive.logged_in") : $t("tools.ytarchive.anonymous")}
            </div>
            <div class="progress"><div class="progress-fill" style:width="{result.total ? (result.done / result.total) * 100 : 0}%"></div></div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result!.dest)}>↗</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{$t("tools.ytarchive.resume_note")}</div><div class="group-row-sub mono">{result.state_path}</div></div>
        </div>
      </div>
    </section>
    <section>
      <div class="group">
        {#each shown as it (it.id)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title"><span class="tag" class:tag-success={it.status === "ok"}>{$t(STATUS_KEY[it.status])}</span> {it.title || it.id}</div>
              <div class="group-row-sub">{it.duration_seconds ? fmtSecs(it.duration_seconds) : "—"}{#if it.error} · {it.error}{/if}</div>
            </div>
            <div class="group-row-trailing">{#if it.file}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(it.file!)}>↗</button>{/if}</div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chk { display: inline-flex; align-items: center; gap: 4px; font-size: var(--text-sm); }
</style>
