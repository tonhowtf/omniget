<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { searchTracks } from "$lib/music/library";
  import { library, loadLibrary } from "$lib/music/library-store.svelte";
  import { ui } from "$lib/music/ui-store.svelte";
  import type { RenamePlan } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { ids = [] }: { ids?: number[] } = $props();

  const PRESETS = ["{album_artist}/{album}/{track} - {title}", "{artist}/{album}/{disc}-{track} {title}", "{artist} - {title}", "{genre}/{artist} - {title}", "{year}/{album_artist} - {album}/{track} {title}"];

  let template = $state(PRESETS[0]);
  let filter = $state("");
  let plan = $state<RenamePlan[]>([]);
  let busy = $state(false);

  let lib = $derived(library());
  let targets = $derived(ids.length ? lib.tracks.filter((x) => ids.includes(x.id)) : filter.trim() ? searchTracks(lib.tracks, filter) : []);
  let changes = $derived(plan.filter((p) => p.from !== p.to));
  let conflicts = $derived(plan.filter((p) => p.conflict));

  async function preview() {
    busy = true;
    try {
      plan = await invoke<RenamePlan[]>("music_rename_preview", { ids: targets.map((x) => x.id), template });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function apply() {
    ui.confirm = {
      title: $t("music.rename_tool.confirm_title") as string,
      body: $t("music.rename_tool.confirm_body", { count: changes.length - conflicts.length }) as string,
      ok: $t("music.rename_tool.apply") as string,
      run: async () => {
        busy = true;
        try {
          const done = await invoke<RenamePlan[]>("music_rename_apply", { ids: targets.map((x) => x.id), template });
          showToast("success", $t("music.rename_tool.done", { count: done.length }) as string);
          plan = [];
          await loadLibrary(true);
        } catch (e) {
          showToast("error", String(e));
        } finally {
          busy = false;
        }
      },
    };
  }

  const rel = (p: string) => p.split(/[\\/]/).slice(-3).join("/");
</script>

<div class="rt">
  <p class="lead">{$t("music.rename_tool.lead")}</p>
  {#if ids.length}
    <p class="dim">{$t("music.convert.selected", { count: targets.length })}</p>
  {:else}
    <input class="input" bind:value={filter} placeholder={$t("music.convert.filter_placeholder") as string} />
    <p class="dim small">{$t("music.convert.matches", { count: targets.length })}</p>
  {/if}
  <label class="field">
    <span class="field-label">{$t("music.convert.naming")}</span>
    <div class="presets">
      {#each PRESETS as p (p)}<button class="btn btn-pill btn-sm" class:active={template === p} onclick={() => (template = p)}>{p}</button>{/each}
    </div>
    <input class="input mono" bind:value={template} />
    <span class="field-hint">{$t("music.convert.tokens")}</span>
  </label>
  <div class="acts">
    <button class="btn" onclick={preview} disabled={!targets.length || busy}><MusicIcon name="search" size={14} /> {$t("music.rename_tool.preview")}</button>
    <button class="btn btn-primary" onclick={apply} disabled={!changes.length || busy}><MusicIcon name="edit" size={14} /> {$t("music.rename_tool.apply")}</button>
    {#if plan.length}<span class="dim small">{$t("music.rename_tool.summary", { changes: changes.length, conflicts: conflicts.length })}</span>{/if}
  </div>
  {#if plan.length}
    <ul class="plan">
      {#each changes.slice(0, 400) as p (p.id)}
        <li class:conflict={p.conflict}>
          <span class="from">{rel(p.from)}</span>
          <MusicIcon name="chevron-right" size={12} />
          <span class="to">{rel(p.to)}</span>
          {#if p.conflict}<span class="tag tag-warning">{$t("music.rename_tool.conflict")}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .rt {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 900px;
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
  }
  .small {
    font-size: var(--text-xs);
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .presets .btn {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .plan {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 420px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .plan li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0;
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .from {
    color: var(--text-muted);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .to {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .conflict .to {
    color: var(--warning);
  }
</style>
