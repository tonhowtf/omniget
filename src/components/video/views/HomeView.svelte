<script lang="ts">
  // Home: only the user's own things. Continue watching (remove in one click),
  // courses in progress and recently added. Sections can be hidden and
  // reordered; the layout lives in localStorage.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import { go, goRoot } from "$lib/video/ui.svelte";
  import type { ContinueEntry, Home, MediaItem, Root, ScanProgress } from "$lib/video/types";
  import ContextMenu, { type MenuItem } from "$components/music/ContextMenu.svelte";
  import { addFolder, itemMenu } from "./parts/actions";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Shelf from "./parts/Shelf.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtDur, lsGet, lsSet, onLibraryChanged, ratio, ready } from "./parts/util";

  type SectionId = "continue" | "courses" | "recent";
  type SectionPref = { id: SectionId; hidden: boolean };
  const ALL: SectionId[] = ["continue", "courses", "recent"];
  const LS_KEY = "omniget.video.home.sections";

  function loadPrefs(): SectionPref[] {
    const saved = lsGet<SectionPref[]>(LS_KEY, []);
    const ok = Array.isArray(saved) ? saved.filter((s) => ALL.includes(s?.id)) : [];
    for (const id of ALL) if (!ok.some((s) => s.id === id)) ok.push({ id, hidden: false });
    return ok;
  }

  let sections = $state<SectionPref[]>(loadPrefs());
  let editing = $state(false);
  let data = $state<Home | null>(null);
  let roots = $state<Root[]>([]);
  let scan = $state<ScanProgress | null>(null);
  let error = $state<string | null>(null);
  let cm = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  async function load() {
    try {
      await ready();
      const [h, s] = await Promise.all([api.home(), api.status()]);
      data = h;
      roots = s.roots;
      error = null;
    } catch (e) {
      error = errText(e);
      data ??= { continue: [], recent: [], courses: [], counts: { movies: 0, shows: 0, courses: 0, videos: 0 } };
    }
  }

  onMount(() => {
    void load();
    const offLib = onLibraryChanged(() => void load());
    let offScan: (() => void) | null = null;
    void listen<ScanProgress>("video-scan-progress", (e) => {
      scan = e.payload.finished ? null : e.payload;
      if (e.payload.finished) void load();
    }).then((u) => (offScan = u));
    return () => {
      offLib();
      offScan?.();
    };
  });

  function savePrefs() {
    lsSet(LS_KEY, sections);
  }
  function move(i: number, dir: -1 | 1) {
    const j = i + dir;
    if (j < 0 || j >= sections.length) return;
    const next = [...sections];
    [next[i], next[j]] = [next[j], next[i]];
    sections = next;
    savePrefs();
  }
  function toggleHidden(i: number) {
    sections[i].hidden = !sections[i].hidden;
    savePrefs();
  }

  async function removeContinue(c: ContinueEntry) {
    if (!data) return;
    const before = data.continue;
    data.continue = before.filter((x) => x.key !== c.key);
    try {
      await api.continueRemove(c.key);
      showToast("info", $t("video.home.removed_from_continue", { title: c.title }) as string);
    } catch (e) {
      data.continue = before;
      showToast("error", errText(e));
    }
  }

  function playCourse(id: number, next: number | null) {
    if (next != null) void openPlayer({ kind: "local", id: next });
    else go({ name: "course", id });
  }

  function recentMenu(e: MouseEvent, it: MediaItem) {
    e.preventDefault();
    cm = { x: e.clientX, y: e.clientY, items: itemMenu(it, () => void load()) };
  }

  let isEmpty = $derived(!!data && !roots.length && !data.continue.length && !data.recent.length && !data.courses.length);
  let hasAny = $derived(!!data && (data.continue.length > 0 || data.courses.length > 0 || data.recent.length > 0));
  let sectionLabel = (id: SectionId) => $t(`video.home.section_${id}`) as string;
</script>

<div class="home">
  <Head title={$t("video.home.title") as string} subtitle={data && hasAny ? ($t("video.home.summary", { movies: data.counts.movies, shows: data.counts.shows, courses: data.counts.courses }) as string) : ""}>
    {#snippet actions()}
      {#if hasAny}
        <button class="btn btn-sm" class:btn-tinted={editing} onclick={() => (editing = !editing)} aria-pressed={editing}>
          <VIcon name="sliders" size={13} />
          {editing ? $t("video.home.done") : $t("video.home.customize")}
        </button>
      {/if}
    {/snippet}
  </Head>

  {#if scan}
    <div class="scan" role="status">
      <span class="spinner"></span>
      <span class="sc-t">{$t("video.home.scanning", { done: scan.done, total: scan.total })}</span>
      <div class="progress"><div class="progress-fill" style:width="{scan.total ? (scan.done / scan.total) * 100 : 0}%"></div></div>
    </div>
  {/if}

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if editing}
    <div class="group edit" aria-label={$t("video.home.customize") as string}>
      {#each sections as s, i (s.id)}
        <div class="group-row erow">
          <span class="en" class:off={s.hidden}>{sectionLabel(s.id)}</span>
          <button class="btn btn-icon btn-sm btn-ghost" disabled={i === 0} onclick={() => move(i, -1)} aria-label={$t("video.home.move_up") as string}><VIcon name="arrow-up" size={14} /></button>
          <button class="btn btn-icon btn-sm btn-ghost" disabled={i === sections.length - 1} onclick={() => move(i, 1)} aria-label={$t("video.home.move_down") as string}><VIcon name="arrow-down" size={14} /></button>
          <button class="toggle toggle-sm" role="switch" aria-checked={!s.hidden} aria-label={$t("video.home.show_section", { name: sectionLabel(s.id) }) as string} onclick={() => toggleHidden(i)}><span class="toggle-knob"></span></button>
        </div>
      {/each}
    </div>
  {/if}

  {#if !data}
    <Skel kind="row" count={5} min={260} />
    <Skel kind="grid" count={8} min={200} />
  {:else if isEmpty}
    <Empty icon="library" title={$t("video.home.empty_title") as string} body={$t("video.home.empty_body") as string}>
      <button class="btn btn-primary btn-lg" onclick={() => void addFolder("library").then((ok) => { if (ok) void load(); })}><VIcon name="folder" size={15} /> {$t("video.home.add_folder")}</button>
      <button class="btn btn-lg" onclick={() => void addFolder("course").then((ok) => { if (ok) void load(); })}><VIcon name="plus" size={15} /> {$t("video.home.import_course")}</button>
      <button class="btn btn-lg" onclick={() => goRoot({ name: "sources" })}><VIcon name="server" size={15} /> {$t("video.home.connect_source")}</button>
    </Empty>
  {:else}
    {#each sections as s (s.id)}
      {#if !s.hidden}
        {#if s.id === "continue" && data.continue.length}
          <Shelf title={sectionLabel("continue")} width={300}>
            {#each data.continue as c (c.key)}
              <article class="cw">
                <button class="cw-open" onclick={() => void openPlayer(c.source)} aria-label={$t("video.common.play_title", { title: c.title }) as string}>
                  <Thumb src={c.thumb} seed={c.title} ratio="16 / 10" radius="var(--radius-xl)" />
                  <span class="cw-shade" aria-hidden="true"></span>
                  <span class="cw-cap">
                    <small>{c.kind === "next" ? $t("video.home.up_next") : c.subtitle}</small>
                    <b>{c.title}</b>
                    {#if c.kind === "next" && c.subtitle}<small>{c.subtitle}</small>{/if}
                  </span>
                  <span class="cw-play" aria-hidden="true"><VIcon name="play" size={16} /></span>
                  {#if c.kind === "resume" && c.progress.duration_ms}
                    <span class="cw-bar" aria-hidden="true"><span style:width="{Math.max(3, (ratio(c.progress) ?? 0) * 100)}%"></span></span>
                    <span class="cw-left">{$t("video.home.time_left", { time: fmtDur(c.progress.duration_ms - c.progress.position_ms) })}</span>
                  {/if}
                </button>
                <button class="cw-rm" onclick={() => void removeContinue(c)} aria-label={$t("video.home.remove_from_continue", { title: c.title }) as string} title={$t("video.home.remove_hint") as string}>
                  <VIcon name="close" size={12} />
                </button>
              </article>
            {/each}
          </Shelf>
        {:else if s.id === "courses" && data.courses.length}
          <Shelf title={sectionLabel("courses")} width={280}>
            {#snippet actions()}
              <button class="btn btn-sm btn-ghost" onclick={() => goRoot({ name: "courses" })}>{$t("video.common.see_all")}</button>
            {/snippet}
            {#each data.courses as c (c.id)}
              {@const pct = c.lessons ? c.watched / c.lessons : 0}
              <article class="course">
                <button class="co-open" onclick={() => go({ name: "course", id: c.id })}>
                  <Thumb src={c.thumb} seed={c.title} icon="list" />
                  <span class="co-meta">
                    <b>{c.title}</b>
                    <small>{$t("video.home.lessons_done", { done: c.watched, total: c.lessons })}</small>
                    <span class="progress"><span class="progress-fill" style:width="{pct * 100}%"></span></span>
                  </span>
                </button>
                <button class="co-play btn btn-sm btn-primary" onclick={() => playCourse(c.id, c.next_lesson)}>
                  <VIcon name="play" size={12} /> {$t("video.home.continue_course")}
                </button>
              </article>
            {/each}
          </Shelf>
        {:else if s.id === "recent" && data.recent.length}
          <section class="recent">
            <h2>{sectionLabel("recent")}</h2>
            <div class="grid">
              {#each data.recent as it (it.id)}
                <MediaCard
                  title={it.show ? it.show : it.title}
                  subtitle={it.show && it.season != null && it.episode != null ? `S${String(it.season).padStart(2, "0")}E${String(it.episode).padStart(2, "0")} · ${it.title}` : fmtDur(it.duration_ms)}
                  src={it.thumb}
                  progress={ratio(it.progress)}
                  watched={!!it.progress?.watched}
                  offline={it.offline}
                  onopen={() => void openPlayer({ kind: "local", id: it.id })}
                  onplay={() => void openPlayer({ kind: "local", id: it.id })}
                  onmenu={(e) => recentMenu(e, it)}
                />
              {/each}
            </div>
          </section>
        {/if}
      {/if}
    {/each}
    {#if !hasAny}
      <Empty compact icon="continue" title={$t("video.home.nothing_yet_title") as string} body={$t("video.home.nothing_yet_body") as string} />
    {/if}
  {/if}
</div>

{#if cm}<ContextMenu x={cm.x} y={cm.y} items={cm.items} onclose={() => (cm = null)} />{/if}

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    padding-bottom: var(--space-7);
    min-width: 0;
  }
  .scan {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-sm);
  }
  .sc-t {
    white-space: nowrap;
  }
  .scan .progress {
    flex: 1;
  }
  .edit {
    max-width: 460px;
  }
  .erow {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .en {
    flex: 1;
    font-weight: 500;
  }
  .en.off {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }
  .cw {
    position: relative;
  }
  .cw-open {
    all: unset;
    position: relative;
    display: block;
    width: 100%;
    border-radius: var(--radius-xl);
    overflow: hidden;
    cursor: pointer;
    box-shadow: var(--elev-1);
    transition:
      transform var(--duration-base) var(--ease-out),
      box-shadow var(--duration-base) var(--ease-out);
  }
  .cw-open:hover {
    transform: scale(1.02);
    box-shadow: var(--elev-2);
  }
  .cw-open:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  .cw-shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(180deg, rgba(0, 0, 0, 0.62) 0%, rgba(0, 0, 0, 0) 45%, rgba(0, 0, 0, 0.55) 100%);
  }
  .cw-cap {
    position: absolute;
    top: 14px;
    left: 16px;
    right: 40px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: white;
  }
  .cw-cap small {
    font-size: var(--text-xs);
    font-weight: 600;
    opacity: 0.85;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cw-cap b {
    font-size: var(--text-lg);
    font-weight: 700;
    line-height: 1.2;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .cw-play {
    position: absolute;
    right: 14px;
    bottom: 22px;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: white;
    color: black;
    transition: transform var(--duration-base) var(--ease-out);
  }
  .cw-open:hover .cw-play {
    transform: scale(1.08);
  }
  .cw-bar {
    position: absolute;
    left: 16px;
    right: 64px;
    bottom: 14px;
    height: 4px;
    border-radius: var(--radius-full);
    background: rgba(255, 255, 255, 0.3);
    overflow: hidden;
  }
  .cw-bar span {
    display: block;
    height: 100%;
    background: white;
  }
  .cw-left {
    position: absolute;
    left: 16px;
    bottom: 24px;
    font-size: var(--text-xs);
    color: rgba(255, 255, 255, 0.85);
  }
  .cw-rm {
    all: unset;
    position: absolute;
    top: 10px;
    right: 10px;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.55);
    color: white;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .cw:hover .cw-rm,
  .cw:focus-within .cw-rm {
    opacity: 1;
  }
  .cw-rm:hover {
    background: rgba(0, 0, 0, 0.8);
  }
  .cw-rm:focus-visible {
    outline: var(--focus-ring);
  }
  .course {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .co-open {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    cursor: pointer;
    min-width: 0;
  }
  .co-open:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-md);
  }
  .co-meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .co-meta b {
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .co-meta small {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .co-play {
    align-self: flex-start;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-4);
  }
  @media (hover: none) {
    .cw-rm {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cw-open,
    .cw-play {
      transition: none;
    }
    .cw-open:hover {
      transform: none;
    }
  }
</style>
