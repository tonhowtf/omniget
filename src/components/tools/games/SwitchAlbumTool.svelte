<script lang="ts">
  /** Importa o álbum do Switch (SD ou pasta) para uma biblioteca por jogo. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, fmtSecs, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    source: string;
    dest: string | null;
    game: string;
    game_id: string;
    known_game: boolean;
    taken_at: string;
    bytes: number;
    kind: string;
    duration_s: number | null;
    width: number | null;
    height: number | null;
    extra: string | null;
    status: string;
    reason: string | null;
  };
  type Game = { game: string; game_id: string; known_game: boolean; clips: number; images: number; bytes: number };
  type Result = {
    items: Item[];
    games: Game[];
    found: number;
    imported: number;
    skipped: number;
    failed: number;
    bytes_imported: number;
    unknown_ids: string[];
    dry_run: boolean;
  };

  let source = $state("");
  let dest = $state("");
  let layout = $state("game");
  let mode = $state("copy");
  let kinds = $state("all");
  let convert = $state("");
  let dedupe = $state(true);
  let dryRun = $state(true);
  let probeClips = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "switch-album") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!source || !dest || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_switch_album", {
        opts: {
          source,
          dest,
          layout,
          mode,
          kinds,
          convert_clips: convert,
          dedupe,
          dry_run: dryRun,
          probe_clips: probeClips,
        },
      });
      showToast("success", `${result.imported} ${$t("tools.switchalbum.imported")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let shown = $derived(result ? result.items.slice(0, 200) : []);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.switchalbum.source")}</div>
          <div class="group-row-sub mono">{source || $t("tools.switchalbum.source_hint")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) source = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.switchalbum.dest")}</div>
          <div class="group-row-sub mono">{dest || $t("tools.switchalbum.dest_hint")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.layout")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={layout}>
            <option value="game">{$t("tools.switchalbum.layout_game")}</option>
            <option value="game-year">{$t("tools.switchalbum.layout_game_year")}</option>
            <option value="year-month">{$t("tools.switchalbum.layout_year_month")}</option>
            <option value="flat">{$t("tools.switchalbum.layout_flat")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.mode")}</div><div class="group-row-sub">{$t("tools.switchalbum.mode_hint")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={mode === "copy"} type="button" onclick={() => (mode = "copy")}>{$t("tools.switchalbum.mode_copy")}</button>
            <button class="segmented-btn" class:active={mode === "move"} type="button" onclick={() => (mode = "move")}>{$t("tools.switchalbum.mode_move")}</button>
          </div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.kinds")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={kinds === "all"} type="button" onclick={() => (kinds = "all")}>{$t("tools.switchalbum.kinds_all")}</button>
            <button class="segmented-btn" class:active={kinds === "images"} type="button" onclick={() => (kinds = "images")}>{$t("tools.switchalbum.kinds_images")}</button>
            <button class="segmented-btn" class:active={kinds === "clips"} type="button" onclick={() => (kinds = "clips")}>{$t("tools.switchalbum.kinds_clips")}</button>
          </div>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.convert")}</div><div class="group-row-sub">{$t("tools.switchalbum.convert_hint")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={convert}>
            <option value="">{$t("tools.switchalbum.convert_none")}</option>
            <option value="webm">WebM</option>
            <option value="gif">GIF</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.dedupe")}</div><div class="group-row-sub">{$t("tools.switchalbum.dedupe_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={dedupe} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.probe")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={probeClips} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.switchalbum.dry_run")}</div><div class="group-row-sub">{$t("tools.switchalbum.dry_run_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={dryRun} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !source || !dest} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.switchalbum.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{result.dry_run ? $t("tools.switchalbum.dry_run_label") : $t("tools.common.done")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.imported} {$t("tools.switchalbum.imported")}</div>
            <div class="group-row-sub">{result.found} {$t("tools.switchalbum.found")} · {result.skipped} {$t("tools.switchalbum.skipped")} · {result.failed} {$t("tools.common.failed")} · <strong>{fmtBytes(result.bytes_imported)}</strong></div>
          </div>
          <div class="group-row-trailing">
            {#if dest}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(dest)}>{$t("tools.common.reveal")}</button>{/if}
          </div>
        </div>
        {#if result.unknown_ids.length}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{result.unknown_ids.length} {$t("tools.switchalbum.unknown")}</div>
              <div class="group-row-sub">{$t("tools.switchalbum.unknown_hint")}</div>
              <div class="group-row-sub mono">{result.unknown_ids.slice(0, 8).join(" · ")}</div>
            </div>
          </div>
        {/if}
      </div>
    </section>

    {#if result.games.length}
      <section>
        <span class="group-label">{$t("tools.switchalbum.games")}</span>
        <div class="group">
          {#each result.games as g (g.game)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{g.game} {#if !g.known_game}<span class="tag">{$t("tools.switchalbum.unknown_tag")}</span>{/if}</div>
                <div class="group-row-sub">{g.images} {$t("tools.switchalbum.images")} · {g.clips} {$t("tools.switchalbum.clips")} · {fmtBytes(g.bytes)}</div>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if shown.length}
      <section>
        <span class="group-label">{$t("tools.switchalbum.files")}</span>
        <div class="group">
          {#each shown as it (it.source)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{it.game} <span class="dim">{it.taken_at}</span></div>
                <div class="group-row-sub mono">{it.dest ? baseName(it.dest) : baseName(it.source)}</div>
                <div class="group-row-sub">
                  {fmtBytes(it.bytes)}
                  {#if it.duration_s}· {fmtSecs(it.duration_s)}{/if}
                  {#if it.width && it.height}· {it.width}×{it.height}{/if}
                  {#if it.status === "imported"}<span class="tag tag-success">{$t("tools.switchalbum.imported")}</span>{/if}
                  {#if it.reason === "duplicate"}<span class="tag">{$t("tools.switchalbum.duplicate")}</span>{/if}
                  {#if it.status === "error"}<span class="tag">{it.reason}</span>{/if}
                </div>
              </div>
              <div class="group-row-trailing">
                {#if it.dest}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(it.dest ?? "")}>↗</button>{/if}
              </div>
            </div>
          {/each}
        </div>
      </section>
    {:else}
      <section><div class="group"><div class="group-row"><div class="group-row-sub">{$t("tools.switchalbum.empty")}</div></div></div></section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
