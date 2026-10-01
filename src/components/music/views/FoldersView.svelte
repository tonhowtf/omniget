<script lang="ts">
  import { t } from "$lib/i18n";
  import { sortTracks } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { go, showTrackMenu } from "$lib/music/ui-store.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import TrackList from "../TrackList.svelte";

  let { path = "" }: { path?: string } = $props();

  let lib = $derived(library());
  let roots = $derived(lib.status?.roots ?? []);
  let sep = $derived(path.includes("\\") && !path.includes("/") ? "\\" : "/");

  function norm(p: string) {
    return p.replace(/[\\/]+$/, "");
  }

  let children = $derived.by(() => {
    void lib.version;
    if (!path) return [];
    const base = norm(path);
    const counts = new Map<string, number>();
    for (const x of lib.tracks) {
      const f = norm(x.folder);
      if (f === base || !f.startsWith(base + "/") && !f.startsWith(base + "\\")) continue;
      const rest = f.slice(base.length + 1);
      const first = rest.split(/[\\/]/)[0];
      counts.set(first, (counts.get(first) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0], undefined, { numeric: true }));
  });
  let here = $derived(sortTracks(lib.tracks.filter((x) => norm(x.folder) === norm(path)), "track"));
  let crumbs = $derived.by(() => {
    if (!path) return [];
    const root = roots.find((r) => norm(path).startsWith(norm(r))) ?? "";
    const rel = norm(path).slice(norm(root).length).split(/[\\/]/).filter(Boolean);
    const out = [{ label: root.split(/[\\/]/).filter(Boolean).pop() ?? root, path: root }];
    let acc = norm(root);
    for (const r of rel) {
      acc = `${acc}${sep}${r}`;
      out.push({ label: r, path: acc });
    }
    return out;
  });
  let selected = $state(new Set<number>());
  let underIds = $derived(
    path ? lib.tracks.filter((x) => norm(x.folder) === norm(path) || norm(x.folder).startsWith(norm(path) + sep)).map((x) => x.id) : [],
  );
</script>

<div class="fv">
  <header>
    <h1>{$t("music.nav.folders")}</h1>
    {#if path}
      <nav class="crumbs" aria-label={$t("music.nav.folders") as string}>
        <button class="crumb" onclick={() => go({ kind: "folders" })}><MusicIcon name="home" size={13} /></button>
        {#each crumbs as c, i (c.path)}
          <MusicIcon name="chevron-right" size={12} />
          <button class="crumb" class:last={i === crumbs.length - 1} onclick={() => go({ kind: "folders", path: c.path })}>{c.label}</button>
        {/each}
      </nav>
      <div class="acts">
        <button class="btn btn-primary btn-sm" disabled={!underIds.length} onclick={() => void playTracks(underIds, undefined, crumbs[crumbs.length - 1]?.label)}><MusicIcon name="play" size={13} /> {$t("music.play_folder")}</button>
      </div>
    {/if}
  </header>

  {#if !path}
    <div class="dirs">
      {#each roots as r (r)}
        <button class="dir" onclick={() => go({ kind: "folders", path: r })}>
          <MusicIcon name="folder" size={22} />
          <span class="dn">{r.split(/[\\/]/).filter(Boolean).pop()}</span>
          <span class="dp">{r}</span>
        </button>
      {:else}
        <p class="dim">{$t("music.no_folders")}</p>
      {/each}
    </div>
  {:else}
    {#if children.length}
      <div class="dirs">
        {#each children as [name, count] (name)}
          <button class="dir" onclick={() => go({ kind: "folders", path: `${norm(path)}${sep}${name}` })}>
            <MusicIcon name="folder" size={20} />
            <span class="dn">{name}</span>
            <span class="dp">{$t("music.count_songs", { count })}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if here.length}
      <div class="list" style:height="{Math.min(here.length, 16) * 46 + 34}px">
        <TrackList tracks={here} bind:selected numbered="track" columns={["album", "quality"]} onplay={(tr) => void playTracks(here.map((x) => x.id), tr.id)} onmenu={(e, sel) => showTrackMenu(e, sel)} />
      </div>
    {/if}
  {/if}
</div>

<style>
  .fv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    color: var(--text-muted);
  }
  .crumb {
    all: unset;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .crumb:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .crumb.last {
    color: var(--text);
    font-weight: 600;
  }
  .crumb:focus-visible {
    outline: var(--focus-ring);
  }
  .acts {
    display: flex;
    gap: var(--space-2);
  }
  .dirs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-2);
  }
  .dir {
    all: unset;
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    column-gap: var(--space-2);
    align-items: center;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    color: var(--accent-hi);
    cursor: pointer;
  }
  .dir :global(svg) {
    grid-row: span 2;
  }
  .dir:hover {
    background: var(--fill-2);
  }
  .dir:focus-visible {
    outline: var(--focus-ring);
  }
  .dn {
    color: var(--text);
    font-weight: 600;
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dp {
    color: var(--text-muted);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dim {
    color: var(--text-dim);
  }
  .list {
    min-height: 120px;
  }
</style>
