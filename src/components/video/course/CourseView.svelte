<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { app } from "$lib/video/app.svelte";
  import { fmtDuration, fmtTime } from "$lib/video/format";
  import { openPlayer, player } from "$lib/video/player.svelte";
  import type { Attachment, CourseDetail, Lesson } from "$lib/video/types";
  import VIcon from "#components/video/player/VIcon.svelte";
  import ProgressRing from "./ProgressRing.svelte";

  let { id }: { id: number } = $props();

  let detail = $state<CourseDetail | null>(null);
  let error = $state<string | null>(null);
  let selecting = $state(false);
  let selected = $state<Set<number>>(new Set());
  let busy = $state(false);

  $effect(() => {
    void app.version;
    if (player.open) return;
    void load(id);
  });

  async function load(courseId: number) {
    try {
      const d = await api.course(courseId);
      if (courseId === id) detail = d;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  let lessons = $derived(detail ? detail.modules.flatMap((m) => m.lessons) : []);
  let total = $derived(lessons.length);
  let done = $derived(lessons.filter((l) => l.progress?.watched).length);
  let frac = $derived(total ? done / total : 0);
  let leftMs = $derived(
    lessons.filter((l) => !l.progress?.watched).reduce((s, l) => s + Math.max(0, l.duration_ms - (l.progress?.position_ms ?? 0)), 0),
  );
  let currentId = $derived(detail?.course.next_lesson ?? null);
  let nextLesson = $derived(lessons.find((l) => l.id === currentId) ?? null);

  function status(l: Lesson): "done" | "current" | "pending" {
    if (l.progress?.watched) return "done";
    if (l.id === currentId) return "current";
    return "pending";
  }

  function durLabel(ms: number): string {
    const d = fmtDuration(ms);
    return d.h ? ($t("video.duration_hm", { hours: d.h, minutes: d.m }) as string) : ($t("video.duration_m", { minutes: d.m }) as string);
  }

  function play(l: Lesson) {
    void openPlayer({ kind: "local", id: l.id }, { title: `${l.number} · ${l.title}` });
  }

  function cont() {
    const l = nextLesson ?? lessons[0];
    if (l) play(l);
  }

  async function mark(keys: string[], watched: boolean) {
    if (!keys.length || busy) return;
    busy = true;
    try {
      await api.setWatched(keys, watched);
      await load(id);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function toggleSel(l: Lesson) {
    const s = new Set(selected);
    if (s.has(l.id)) s.delete(l.id);
    else s.add(l.id);
    selected = s;
  }

  function selectedKeys(): string[] {
    return lessons.filter((l) => selected.has(l.id)).map((l) => l.key);
  }

  async function markSelected(watched: boolean) {
    await mark(selectedKeys(), watched);
    selected = new Set();
    selecting = false;
  }

  async function exportNotes() {
    try {
      const path = await api.notesExport({ courseId: id });
      showToast("success", $t("video.notes.exported", { path }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function openAttachment(a: Attachment) {
    try {
      await invoke("open_path_default", { path: a.path });
    } catch (e) {
      showToast("error", String(e));
    }
  }

  const ATT_ICON: Record<Attachment["kind"], string> = {
    pdf: "notes",
    code: "transcript",
    subtitle: "captions",
    archive: "folder",
    image: "camera",
    doc: "notes",
    other: "paperclip",
  };
</script>

{#if error}
  <p class="err">{error}</p>
{:else if !detail}
  <div class="loading"><VIcon name="loader" size={22} spin /></div>
{:else}
  {@const c = detail.course}
  <div class="course">
    <header class="hero">
      <ProgressRing value={frac} size={76} stroke={6} label={$t("video.courses.progress", { percent: Math.round(frac * 100) }) as string} />
      <div class="ht">
        <h1>{c.title}</h1>
        <p class="meta">
          {$t("video.courses.lessons_done", { done, total })} · {$t("video.courses.time_left", { time: durLabel(leftMs) })}
        </p>
        <p class="path" title={c.root_path}>{c.root_path}</p>
      </div>
      <div class="ha">
        {#if total}
          <button class="btn btn-primary" onclick={cont}>
            <VIcon name="play" size={12} />
            {#if nextLesson}
              <span class="nl">{$t(done ? "video.courses.continue_with" : "video.courses.start_with", { lesson: nextLesson.number })}</span>
            {:else}{$t("video.courses.review")}{/if}
          </button>
        {/if}
        <button class="btn" onclick={exportNotes}><VIcon name="export" size={13} /> {$t("video.courses.export_notes")}</button>
        <button
          class="btn"
          class:active={selecting}
          aria-pressed={selecting}
          onclick={() => {
            selecting = !selecting;
            selected = new Set();
          }}><VIcon name="check" size={13} /> {$t("video.courses.select")}</button
        >
      </div>
    </header>

    {#if selecting}
      <div class="batch" role="toolbar" aria-label={$t("video.courses.batch") as string}>
        <span>{$t("video.courses.selected", { count: selected.size })}</span>
        <button class="btn btn-sm" onclick={() => (selected = new Set(lessons.map((l) => l.id)))}>{$t("video.courses.select_all")}</button>
        <button class="btn btn-sm" onclick={() => (selected = new Set())}>{$t("video.courses.select_none")}</button>
        <span class="sp"></span>
        <button class="btn btn-sm btn-primary" disabled={!selected.size || busy} onclick={() => markSelected(true)}>{$t("video.courses.mark_watched")}</button>
        <button class="btn btn-sm" disabled={!selected.size || busy} onclick={() => markSelected(false)}>{$t("video.courses.mark_unwatched")}</button>
      </div>
    {/if}

    {#if detail.loose_attachments.length}
      <section class="materials">
        <h2>{$t("video.courses.materials")}</h2>
        <div class="atts">
          {#each detail.loose_attachments as a (a.path)}
            <button class="att" onclick={() => openAttachment(a)} title={a.path}><VIcon name={ATT_ICON[a.kind]} size={12} /> {a.name}</button>
          {/each}
        </div>
      </section>
    {/if}

    {#each detail.modules as mod (mod.index)}
      {@const mfrac = mod.lessons.length ? mod.watched / mod.lessons.length : 0}
      <details class="mod" open={mfrac < 1 || detail.modules.length === 1}>
        <summary>
          <VIcon name="chevron" size={14} class="chev" />
          <span class="mt">{mod.title}</span>
          <span class="mbar" aria-hidden="true"><span style:width="{mfrac * 100}%"></span></span>
          <span class="mc">{mod.watched}/{mod.lessons.length}</span>
          <span class="md">{durLabel(mod.duration_ms)}</span>
          <button
            class="ic"
            onclick={(e) => {
              e.preventDefault();
              void mark(
                mod.lessons.map((l) => l.key),
                mfrac < 1,
              );
            }}
            title={(mfrac < 1 ? $t("video.courses.mark_module") : $t("video.courses.unmark_module")) as string}
            aria-label={(mfrac < 1 ? $t("video.courses.mark_module") : $t("video.courses.unmark_module")) as string}
          >
            <VIcon name={mfrac < 1 ? "check" : "replay"} size={13} />
          </button>
        </summary>
        <ol class="steps">
          {#each mod.lessons as l (l.id)}
            {@const st = status(l)}
            {@const partial = !l.progress?.watched && l.progress && l.duration_ms ? l.progress.position_ms / l.duration_ms : 0}
            <li class="step {st}" aria-current={st === "current" ? "step" : undefined}>
              {#if selecting}
                <input type="checkbox" checked={selected.has(l.id)} onchange={() => toggleSel(l)} aria-label={`${l.number} ${l.title}`} />
              {/if}
              <button
                class="state"
                onclick={() => mark([l.key], !l.progress?.watched)}
                title={(l.progress?.watched ? $t("video.courses.mark_unwatched") : $t("video.courses.mark_watched")) as string}
                aria-label={(l.progress?.watched ? $t("video.courses.mark_unwatched") : $t("video.courses.mark_watched")) as string}
              >
                {#if st === "done"}<VIcon name="check" size={11} />{:else if st === "current"}<span class="pulse"></span>{/if}
              </button>
              <button class="main" onclick={() => (selecting ? toggleSel(l) : play(l))}>
                <span class="num">{l.number}</span>
                <span class="lt">{l.title}</span>
                {#if partial > 0.01}
                  <span class="pbar" aria-hidden="true"><span style:width="{Math.min(100, partial * 100)}%"></span></span>
                {/if}
                <span class="du">{l.duration_ms ? fmtTime(l.duration_ms) : ""}</span>
              </button>
              {#if l.attachments.length}
                <div class="atts inline">
                  {#each l.attachments as a (a.path)}
                    <button class="att" onclick={() => openAttachment(a)} title={a.path}><VIcon name={ATT_ICON[a.kind]} size={11} /> {a.name}</button>
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ol>
      </details>
    {/each}
  </div>
{/if}

<style>
  .course {
    max-width: 980px;
    padding-bottom: var(--space-7);
  }
  .loading {
    display: grid;
    place-items: center;
    height: 50%;
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
  }
  .hero {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
    flex-wrap: wrap;
  }
  .ht {
    flex: 1;
    min-width: 240px;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    letter-spacing: var(--track-tight);
    line-height: var(--leading-2xl);
  }
  .meta {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .path {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 520px;
  }
  .ha {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .nl {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .batch {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: var(--space-3);
    padding: 8px 12px;
    border-radius: var(--radius-lg);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-2);
    font-size: var(--text-sm);
  }
  .sp {
    flex: 1;
  }
  .materials {
    margin-bottom: var(--space-3);
  }
  .materials h2 {
    margin: 0 0 6px;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--text-muted);
  }
  .atts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .atts.inline {
    width: 100%;
    padding: 0 0 4px 62px;
  }
  .att {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 260px;
    padding: 3px 9px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-xs);
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .att:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .mod {
    margin-bottom: var(--space-2);
    border-radius: var(--radius-xl);
    background: var(--surface-mut);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  summary {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    list-style: none;
    cursor: pointer;
    font-weight: 650;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary :global(.chev) {
    color: var(--text-muted);
    transform: rotate(-90deg);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  details[open] > summary :global(.chev) {
    transform: none;
  }
  .mt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mbar,
  .pbar {
    width: 72px;
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
    flex-shrink: 0;
  }
  .mbar span,
  .pbar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .pbar {
    width: 48px;
  }
  .mc,
  .md {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .ic:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0 8px 10px;
  }
  .step {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 0 6px;
    border-radius: var(--radius-md);
  }
  .step:hover {
    background: var(--fill-1);
  }
  .step.current {
    background: var(--accent-soft);
  }
  .step input {
    accent-color: var(--accent);
  }
  .state {
    all: unset;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 6px;
    box-shadow: inset 0 0 0 1.5px var(--fill-4);
    cursor: pointer;
    transition:
      background var(--duration-fast),
      transform var(--duration-fast) var(--ease-spring);
  }
  .state:hover {
    box-shadow: inset 0 0 0 1.5px var(--success);
  }
  .state:active {
    transform: scale(0.85);
  }
  .done .state {
    box-shadow: none;
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }
  .current .state {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse-dot 1.6s var(--ease-in-out) infinite;
  }
  @keyframes pulse-dot {
    50% {
      transform: scale(0.6);
      opacity: 0.6;
    }
  }
  .main {
    all: unset;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    cursor: pointer;
    font-size: var(--text-base);
  }
  .num {
    min-width: 36px;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .lt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pending .lt {
    color: var(--text);
  }
  .done .lt {
    color: var(--text-dim);
  }
  .current .lt {
    font-weight: 650;
  }
  .du {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .main:focus-visible,
  .state:focus-visible,
  .att:focus-visible,
  .ic:focus-visible {
    outline: var(--focus-ring);
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
