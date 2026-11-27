<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, proxiedImage } from "$lib/read/library-store.svelte";
  import { startBookDownload, readDownloads } from "$lib/read/downloads.svelte";
  import { go, openBook, tr } from "$lib/read/ui-store.svelte";
  import type { Download, Hit } from "$lib/read/types";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  let { hit }: { hit: Hit } = $props();
  let lib = $derived(library());
  let hydrated = $state<Hit | null>(null);
  let loadingDetails = $state(false);
  let detailError = $state("");

  let activeHit = $derived(hydrated && hydrated.source === hit.source && hydrated.key === hit.key ? hydrated : hit);

  $effect(() => {
    const src = hit.source;
    const key = hit.key;
    const needsDetails = hit.downloads.length === 0 && (src === "archiveorg" || src.startsWith("decl:"));
    hydrated = null;
    detailError = "";
    if (!needsDetails) {
      loadingDetails = false;
      return;
    }
    let cancelled = false;
    loadingDetails = true;
    void invoke<Hit>("read_hit_details", { source: src, key })
      .then((full) => {
        if (cancelled) return;
        hydrated = {
          ...hit,
          ...full,
          title: full.title || hit.title,
          authors: full.authors.length ? full.authors : hit.authors,
          cover: full.cover || hit.cover,
          summary: full.summary || hit.summary,
          subjects: full.subjects.length ? full.subjects : hit.subjects,
          downloads: full.downloads.length ? full.downloads : hit.downloads,
        };
      })
      .catch((e) => {
        if (!cancelled) detailError = String(e);
      })
      .finally(() => {
        if (!cancelled) loadingDetails = false;
      });
    return () => {
      cancelled = true;
    };
  });

  let existing = $derived(lib.books.find((b) => b.source_url && activeHit.downloads.some((d) => d.url === b.source_url)));
  let running = $derived(readDownloads.items.find((d) => d.state === "running" && activeHit.downloads.some((x) => x.url === d.url)));
  let done = $derived(readDownloads.items.find((d) => d.state === "done" && activeHit.downloads.some((x) => x.url === d.url)));

  async function get(d: Download) {
    try {
      await startBookDownload(d.url, activeHit.title, activeHit.source, activeHit.cover);
      showToast("success", tr("read.hit.downloading", { title: activeHit.title }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  let best = $derived(activeHit.downloads.find((d) => d.format === "epub") ?? activeHit.downloads[0]);
</script>

<div class="hv">
  <div class="cv"><Cover src={activeHit.source.startsWith("decl:") ? proxiedImage(activeHit.cover, activeHit.url) : (activeHit.cover ?? "")} title={activeHit.title} radius={8} /></div>
  <div class="info">
    <h1>{activeHit.title}</h1>
    <p class="au">{activeHit.authors.join(", ")}{activeHit.year ? ` · ${activeHit.year}` : ""} {#if activeHit.language}<span class="r-badge">{activeHit.language}</span>{/if}</p>
    <div class="r-line">
      {#if existing}
        <button class="btn btn-primary" onclick={() => openBook(existing!)}><ReadIcon name="book-open" size={15} /> {$t("read.book.read")}</button>
        <button class="btn" onclick={() => go({ kind: "book", id: existing!.id })}>{$t("read.book.details")}</button>
      {:else if done?.bookIds?.length}
        <button class="btn btn-primary" onclick={() => openBook({ id: done!.bookIds![0] })}><ReadIcon name="book-open" size={15} /> {$t("read.book.read")}</button>
      {:else if running}
        <button class="btn" disabled><ReadIcon name="refresh" size={15} spin /> {$t("read.hit.in_progress")}</button>
      {:else if loadingDetails}
        <button class="btn" disabled><ReadIcon name="refresh" size={15} spin /> {$t("read.hit.in_progress")}</button>
      {:else if best}
        <button class="btn btn-primary" onclick={() => get(best!)}><ReadIcon name="download" size={15} /> {$t("read.hit.get", { format: best.label })}</button>
      {:else}
        <p class="r-muted">{detailError || $t("read.hit.no_files")}</p>
      {/if}
      {#if activeHit.url}<button class="btn" onclick={() => openUrl(activeHit.url)}><ReadIcon name="external" size={15} /> {$t("read.hit.open_site")}</button>{/if}
    </div>
    {#if activeHit.downloads.length > 1}
      <h2 class="r-sub">{$t("read.hit.formats")}</h2>
      <div class="r-line">
        {#each activeHit.downloads as d (d.url)}
          <button class="r-chip" onclick={() => get(d)}><ReadIcon name="download" size={13} /> {d.label}</button>
        {/each}
      </div>
    {/if}
    {#if activeHit.summary}<p class="desc">{activeHit.summary.replace(/<[^>]+>/g, "")}</p>{/if}
    {#if activeHit.subjects.length}
      <div class="r-line">{#each activeHit.subjects.slice(0, 18) as s (s)}<span class="r-chip">{s}</span>{/each}</div>
    {/if}
    {#if activeHit.extra?.downloads}<p class="r-muted">{$t("read.hit.popularity", { count: activeHit.extra.downloads })}</p>{/if}
  </div>
</div>

<style>
  .hv {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: var(--space-5);
    margin-top: var(--space-3);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
  }
  .au {
    color: var(--text-dim);
    font-size: var(--text-lg);
    margin: 4px 0 var(--space-3);
  }
  .desc {
    white-space: pre-line;
    line-height: 1.55;
    max-width: 75ch;
    color: var(--text-dim);
  }
  .r-line .r-chip {
    cursor: pointer;
  }
  @media (max-width: 700px) {
    .hv {
      grid-template-columns: 120px 1fr;
    }
  }
</style>
