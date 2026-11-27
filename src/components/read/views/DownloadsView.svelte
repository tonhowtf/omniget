<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { clearFinishedReadDownloads, readDownloads, startBookDownload } from "$lib/read/downloads.svelte";
  import { library } from "$lib/read/library-store.svelte";
  import { go, openBook, relTime } from "$lib/read/ui-store.svelte";
  import type { ConvertJob, QueueItem } from "$lib/read/types";
  import ReadIcon from "../ReadIcon.svelte";

  let lib = $derived(library());
  let jobs = $state<ConvertJob[]>([]);
  let dlMap = $derived(getDownloads());

  onMount(() => {
    void invoke<QueueItem[]>("read_download_queue").then((q) => (lib.queue = q));
    void invoke<ConvertJob[]>("read_convert_jobs").then((j) => (jobs = j));
    const un = listen<ConvertJob>("read-convert-progress", (e) => {
      const i = jobs.findIndex((j) => j.id === e.payload.id);
      if (i >= 0) jobs[i] = e.payload;
      else jobs = [e.payload, ...jobs];
    });
    return () => void un.then((f) => f());
  });

  let active = $derived(lib.queue.filter((q) => q.state === "running" || q.state === "queued"));
  let allEmpty = $derived(!lib.queue.length && !readDownloads.items.length && !jobs.length);

  async function retryChapter(chapterId: number) {
    try {
      await invoke("read_chapters_download", { ids: [chapterId] });
      lib.queue = await invoke<QueueItem[]>("read_download_queue");
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function cancelBook(id: number) {
    try {
      await invoke("cancel_generic_download", { downloadId: id });
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function retryBook(url: string, title: string, source: string, cover?: string | null) {
    try {
      await startBookDownload(url, title, source, cover ?? null);
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<div class="r-h">
  <h1>{$t("read.nav.downloads")}</h1>
  <span class="spacer"></span>
  <button class="r-chip" onclick={() => goto("/downloads")}>{$t("read.downloads.open_queue")}</button>
</div>

{#if allEmpty}
  <div class="empty-state">
    <ReadIcon name="download" size={24} />
    <p class="r-muted">{$t("read.downloads.none")}</p>
    <button class="btn btn-primary" onclick={() => go({ kind: "browse" })}>{$t("read.nav.browse")}</button>
  </div>
{:else}
  {#if lib.queue.length}
    <h2 class="r-sub">
      {$t("read.downloads.chapters")}
      <span class="spacer"></span>
      {#if active.length}<button class="r-chip" onclick={() => invoke("read_download_cancel", { clearFinished: false })}>{$t("read.downloads.cancel_all")}</button>{/if}
      <button class="r-chip" onclick={async () => { await invoke("read_download_cancel", { clearFinished: true }); lib.queue = await invoke<QueueItem[]>("read_download_queue"); }}>{$t("read.downloads.clear")}</button>
    </h2>
    <ul class="r-list">
      {#each lib.queue as q (q.chapter_id)}
        <li class="r-li">
          <ReadIcon name={q.state === "done" ? "check" : q.state === "failed" ? "alert" : "download"} size={15} spin={false} />
          <div class="it">
            <b>{q.series_title}</b> <span>{q.chapter_title}{#if q.state === "running" && q.total} · {q.done}/{q.total}{/if}</span>
            {#if q.state === "running" && q.total}<div class="r-bar"><i style:width="{Math.round((q.done / q.total) * 100)}%"></i></div>{/if}
            {#if q.error}<span class="err">{q.error}</span>{/if}
          </div>
          {#if q.state === "failed"}
            <button class="btn" onclick={() => retryChapter(q.chapter_id)}>{$t("downloads.retry")}</button>
          {/if}
          <span class="r-badge">{$t(`read.downloads.state_${q.state}`)}</span>
        </li>
      {/each}
    </ul>
  {/if}

  {#if readDownloads.items.length}
    <h2 class="r-sub">
      {$t("read.downloads.books")}
      <span class="spacer"></span>
      {#if readDownloads.items.some((d) => d.state !== "running")}<button class="r-chip" onclick={clearFinishedReadDownloads}>{$t("read.downloads.clear")}</button>{/if}
    </h2>
    <ul class="r-list">
      {#each readDownloads.items as d (d.id)}
        {@const live = dlMap.get(d.id)}
        {@const pct = live?.percent ?? 0}
        <li class="r-li">
          <ReadIcon name={d.state === "done" ? "check" : d.state === "failed" ? "alert" : "download"} size={15} />
          <div class="it">
            <b>{d.title}</b>
            <span class="r-muted">
              {d.source} · {relTime(d.at)}
              {#if d.state === "running" && live}
                {#if pct > 0} · {Math.round(pct)}%{/if}
                {#if live.speed} · {live.speed}{/if}
              {/if}
            </span>
            {#if d.state === "running" && live && pct > 0}
              <div class="r-bar"><i style:width="{Math.min(100, Math.round(pct))}%"></i></div>
            {/if}
            {#if d.error}<span class="err">{d.error}</span>{/if}
          </div>
          {#if d.state === "running"}
            <button class="btn" onclick={() => cancelBook(d.id)}>{$t("downloads.cancel")}</button>
          {:else if d.state === "failed"}
            <button class="btn" onclick={() => retryBook(d.url, d.title, d.source, d.cover)}>{$t("downloads.retry")}</button>
          {:else if d.bookIds?.length}
            <button class="btn btn-primary" onclick={() => openBook({ id: d.bookIds![0] })}>{$t("read.book.read")}</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if jobs.length}
    <h2 class="r-sub">
      {$t("read.downloads.conversions")}
      <span class="spacer"></span>
      {#if jobs.some((j) => j.state !== "running")}<button class="r-chip" onclick={async () => { await invoke("read_convert_clear"); jobs = await invoke<ConvertJob[]>("read_convert_jobs"); }}>{$t("read.downloads.clear")}</button>{/if}
    </h2>
    <ul class="r-list">
      {#each jobs as j (j.id)}
        <li class="r-li">
          <ReadIcon name={j.state === "done" ? "check" : j.state === "failed" ? "alert" : "convert"} size={15} />
          <div class="it">
            <b>{j.label}</b>
            {#if j.state === "running"}<div class="r-bar"><i style:width="{Math.round(j.progress * 100)}%"></i></div>{/if}
            {#if j.message}<span class={j.state === "failed" ? "err" : "r-muted"}>{j.message}</span>{/if}
          </div>
          {#if j.book_id}<button class="btn" onclick={() => go({ kind: "book", id: j.book_id! })}>{$t("read.book.details")}</button>{/if}
          {#if j.output}<button class="r-icon-btn" onclick={() => invoke("reveal_file", { path: j.output })} aria-label={$t("read.book.reveal") as string}><ReadIcon name="folder" size={15} /></button>{/if}
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .spacer {
    flex: 1;
  }
  .it {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .it span {
    font-size: var(--text-sm);
  }
  .it .err {
    color: var(--error, var(--danger));
    font-size: var(--text-xs);
  }
</style>
