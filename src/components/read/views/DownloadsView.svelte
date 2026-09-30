<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { clearFinishedReadDownloads, readDownloads } from "$lib/read/downloads.svelte";
  import { library } from "$lib/read/library-store.svelte";
  import { go, openBook, relTime } from "$lib/read/ui-store.svelte";
  import type { ConvertJob, QueueItem } from "$lib/read/types";
  import ReadIcon from "../ReadIcon.svelte";

  let lib = $derived(library());
  let jobs = $state<ConvertJob[]>([]);

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
</script>

<div class="r-h">
  <h1>{$t("read.nav.downloads")}</h1>
</div>

<h2 class="r-sub">
  {$t("read.downloads.chapters")}
  <span style="flex:1"></span>
  {#if active.length}<button class="r-chip" onclick={() => invoke("read_download_cancel", { clearFinished: false })}>{$t("read.downloads.cancel_all")}</button>{/if}
  {#if lib.queue.length}<button class="r-chip" onclick={async () => { await invoke("read_download_cancel", { clearFinished: true }); lib.queue = await invoke<QueueItem[]>("read_download_queue"); }}>{$t("read.downloads.clear")}</button>{/if}
</h2>
{#if !lib.queue.length}<p class="r-muted">{$t("read.downloads.none")}</p>{/if}
<ul class="r-list">
  {#each lib.queue as q (q.chapter_id)}
    <li class="r-li">
      <ReadIcon name={q.state === "done" ? "check" : q.state === "failed" ? "alert" : "download"} size={15} spin={false} />
      <div class="it">
        <b>{q.series_title}</b> <span>{q.chapter_title}</span>
        {#if q.state === "running" && q.total}<div class="r-bar"><i style:width="{Math.round((q.done / q.total) * 100)}%"></i></div>{/if}
        {#if q.error}<span class="err">{q.error}</span>{/if}
      </div>
      <span class="r-badge">{$t(`read.downloads.state_${q.state}`)}</span>
    </li>
  {/each}
</ul>

<h2 class="r-sub">
  {$t("read.downloads.books")}
  <span style="flex:1"></span>
  {#if readDownloads.items.some((d) => d.state !== "running")}<button class="r-chip" onclick={clearFinishedReadDownloads}>{$t("read.downloads.clear")}</button>{/if}
  <button class="r-chip" onclick={() => goto("/downloads")}>{$t("read.downloads.open_queue")}</button>
</h2>
{#if !readDownloads.items.length}<p class="r-muted">{$t("read.downloads.none")}</p>{/if}
<ul class="r-list">
  {#each readDownloads.items as d (d.id)}
    <li class="r-li">
      <ReadIcon name={d.state === "done" ? "check" : d.state === "failed" ? "alert" : "download"} size={15} />
      <div class="it"><b>{d.title}</b><span class="r-muted">{d.source} · {relTime(d.at)}</span>{#if d.error}<span class="err">{d.error}</span>{/if}</div>
      {#if d.bookIds?.length}<button class="btn" onclick={() => openBook({ id: d.bookIds![0] })}>{$t("read.book.read")}</button>{/if}
    </li>
  {/each}
</ul>

<h2 class="r-sub">
  {$t("read.downloads.conversions")}
  <span style="flex:1"></span>
  {#if jobs.some((j) => j.state !== "running")}<button class="r-chip" onclick={async () => { await invoke("read_convert_clear"); jobs = await invoke<ConvertJob[]>("read_convert_jobs"); }}>{$t("read.downloads.clear")}</button>{/if}
</h2>
{#if !jobs.length}<p class="r-muted">{$t("read.downloads.none")}</p>{/if}
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

<style>
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
  .err {
    color: var(--danger);
    font-size: var(--text-xs) !important;
  }
</style>
