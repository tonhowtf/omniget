<script lang="ts">
  import { pluralKey } from "$lib/home/plural";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { app } from "$lib/video/app.svelte";
  import { fmtDuration } from "$lib/video/format";
  import { openPlayer } from "$lib/video/player.svelte";
  import type { CourseSummary } from "$lib/video/types";
  import { go } from "$lib/video/ui.svelte";
  import VIcon from "#components/video/player/VIcon.svelte";
  import ProgressRing from "./ProgressRing.svelte";

  let list = $state<CourseSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let importing = $state(false);
  let confirmRemove = $state<CourseSummary | null>(null);

  $effect(() => {
    void app.version;
    void load();
  });

  async function load() {
    try {
      list = await api.courses();
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function importFolder() {
    let path: string | string[] | null = null;
    try {
      path = await openDialog({ directory: true, multiple: false, title: $t("video.courses.import_title") as string });
    } catch {
      return;
    }
    if (!path || Array.isArray(path)) return;
    importing = true;
    try {
      const c = await api.courseImport(path);
      showToast("success", $t("video.courses.imported", { title: c.title }) as string);
      await load();
      go({ name: "course", id: c.id });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      importing = false;
    }
  }

  async function remove(c: CourseSummary) {
    confirmRemove = null;
    try {
      await api.courseRemove(c.id);
      list = list.filter((x) => x.id !== c.id);
      showToast("success", $t("video.courses.removed", { title: c.title }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function resume(c: CourseSummary) {
    if (c.next_lesson != null) void openPlayer({ kind: "local", id: c.next_lesson });
    else go({ name: "course", id: c.id });
  }

  function durLabel(ms: number): string {
    const d = fmtDuration(ms);
    return d.h ? ($t("video.duration_hm", { hours: d.h, minutes: d.m }) as string) : ($t("video.duration_m", { minutes: d.m }) as string);
  }
</script>

<div class="courses">
  <header>
    <div>
      <h1>{$t("video.courses.title")}</h1>
      {#if list.length}<p class="sub">{$t(pluralKey("video.courses.count", list.length), { count: list.length })}</p>{/if}
    </div>
    <button class="btn btn-primary" onclick={importFolder} disabled={importing}>
      <VIcon name={importing ? "loader" : "plus"} size={14} spin={importing} />
      {$t("video.courses.import")}
    </button>
  </header>

  {#if loading}
    <div class="grid">
      {#each Array(4) as _, i (i)}<div class="card skel"></div>{/each}
    </div>
  {:else if error}
    <p class="err">{error}</p>
  {:else if !list.length}
    <div class="empty">
      <img src="/icons/video/courses-128.webp" alt="" width="84" height="84" />
      <h2>{$t("video.courses.empty_title")}</h2>
      <p>{$t("video.courses.empty_hint")}</p>
      <button class="btn btn-primary" onclick={importFolder} disabled={importing}>{$t("video.courses.import")}</button>
    </div>
  {:else}
    <div class="grid">
      {#each list as c (c.id)}
        {@const frac = c.lessons ? c.watched / c.lessons : 0}
        {@const thumb = api.serverUrl(c.thumb)}
        <article class="card">
          <button class="cover" onclick={() => go({ name: "course", id: c.id })} aria-label={c.title}>
            {#if thumb}<img src={thumb} alt="" loading="lazy" />{:else}<img class="ph" src="/icons/video/courses-128.webp" alt="" />{/if}
            <span class="bar"><span style:width="{frac * 100}%"></span></span>
          </button>
          <div class="info">
            <div class="txt">
              <button class="title" onclick={() => go({ name: "course", id: c.id })}>{c.title}</button>
              <span class="meta">
                {$t("video.courses.lessons_done", { done: c.watched, total: c.lessons })} · {durLabel(c.duration_ms)}
              </span>
            </div>
            <ProgressRing value={frac} size={40} label={$t("video.courses.progress", { percent: Math.round(frac * 100) }) as string} />
          </div>
          <div class="acts">
            <button class="btn btn-sm btn-primary" onclick={() => resume(c)}>
              <VIcon name="play" size={11} />
              {frac >= 1 ? $t("video.courses.review") : c.watched ? $t("video.courses.continue") : $t("video.courses.start")}
            </button>
            <span class="sp"></span>
            <button class="ic" onclick={() => (confirmRemove = c)} aria-label={$t("video.courses.remove") as string} title={$t("video.courses.remove") as string}>
              <VIcon name="trash" size={14} />
            </button>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

{#if confirmRemove}
  {@const c = confirmRemove}
  <div class="dialog-backdrop" role="presentation" onclick={() => (confirmRemove = null)}>
    <div class="dialog-card" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (confirmRemove = null)}>
      <h3 class="dialog-title">{$t("video.courses.remove_title", { title: c.title })}</h3>
      <p class="dialog-body">{$t("video.courses.remove_body")}</p>
      <div class="dialog-actions">
        <button class="btn" onclick={() => (confirmRemove = null)}>{$t("video.cancel")}</button>
        <button class="btn btn-destructive" onclick={() => remove(c)}>{$t("video.courses.remove")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .courses {
    padding-bottom: var(--space-6);
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    letter-spacing: var(--track-tight);
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-4);
  }
  .card {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: var(--elev-1);
    overflow: hidden;
    transition:
      transform var(--duration-base) var(--ease-out),
      box-shadow var(--duration-base);
  }
  .card:hover {
    transform: translateY(-2px);
    box-shadow: var(--elev-2);
  }
  .skel {
    height: 250px;
    background: var(--fill-1);
    box-shadow: none;
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.5;
    }
  }
  .cover {
    all: unset;
    position: relative;
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    background: var(--fill-1);
    cursor: pointer;
    overflow: hidden;
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .cover img.ph {
    width: 64px;
    height: 64px;
    object-fit: contain;
    opacity: 0.9;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 4px;
    background: color-mix(in srgb, var(--bg) 60%, transparent);
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .info {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px 4px;
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    all: unset;
    font-weight: 650;
    font-size: var(--text-md);
    cursor: pointer;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .title:hover {
    color: var(--accent-hi);
  }
  .meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px 14px;
  }
  .sp {
    flex: 1;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .ic:hover {
    background: var(--fill-2);
    color: var(--danger);
  }
  .cover:focus-visible,
  .title:focus-visible,
  .ic:focus-visible {
    outline: var(--focus-ring);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-8) auto;
    max-width: 420px;
    text-align: center;
  }
  .empty h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-lg);
  }
  .empty p {
    margin: 0 0 var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .err {
    color: var(--danger);
  }
  @media (prefers-reduced-motion: reduce) {
    .card,
    .skel {
      transition: none;
      animation: none;
    }
  }
</style>
