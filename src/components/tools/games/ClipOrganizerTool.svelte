<script lang="ts">
  /** Clipes e prints do PC (Game Bar, ShadowPlay, OBS, Steam) agrupados por jogo. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    source: string;
    dest: string | null;
    game: string;
    game_id: string;
    known_game: boolean;
    taken_at: string;
    bytes: number;
    kind: string;
    extra: string | null;
    status: string;
    reason: string | null;
  };
  type Game = { game: string; game_id: string; known_game: boolean; clips: number; images: number; bytes: number };
  type Big = { path: string; game: string; bytes: number };
  type Compressed = { input: string; output: string | null; bytes_before: number; bytes_after: number; ok: boolean; error: string | null };
  type Result = {
    items: Item[];
    games: Game[];
    found: number;
    imported: number;
    skipped: number;
    failed: number;
    bytes_total: number;
    largest: Big[];
    recompressed: Compressed[];
    mode: string;
    dry_run: boolean;
  };

  let dirs = $state<string[]>([]);
  let steamDirs = $state<string[]>([]);
  let includeSteam = $state(true);
  let dest = $state("");
  let layout = $state("game");
  let mode = $state("scan");
  let dedupe = $state(true);
  let dryRun = $state(false);
  let recompressOver = $state(0);
  let targetMb = $state(25);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "clip-organizer" || p.id === "video-compress") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  let organizing = $derived(mode === "copy" || mode === "move");
  let ready = $derived((dirs.length > 0 || includeSteam) && (!organizing || !!dest));

  async function run() {
    if (busy || !ready) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_clip_organizer", {
        opts: {
          dirs,
          include_steam: includeSteam,
          steam_dirs: steamDirs,
          dest,
          layout,
          mode,
          dedupe,
          dry_run: dryRun,
          recompress_over_mb: recompressOver,
          target_mb: targetMb,
          fallback_game: $t("tools.cliporg.fallback"),
        },
      });
      showToast("success", `${result.found} ${$t("tools.cliporg.found")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  let saved = $derived(
    result ? result.recompressed.filter((c) => c.ok).reduce((n, c) => n + Math.max(0, c.bytes_before - c.bytes_after), 0) : 0,
  );
  let shown = $derived(result ? result.items.slice(0, 200) : []);
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.cliporg.folders")}</div>
          <div class="group-row-sub mono">{dirs.join(" · ") || $t("tools.cliporg.folders_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if dirs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dirs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d && !dirs.includes(d)) dirs = [...dirs, d]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.include_steam")}</div><div class="group-row-sub">{$t("tools.cliporg.include_steam_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={includeSteam} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.cliporg.steam_dirs")}</div>
          <div class="group-row-sub mono">{steamDirs.join(" · ") || $t("tools.cliporg.steam_dirs_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if steamDirs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (steamDirs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d && !steamDirs.includes(d)) steamDirs = [...steamDirs, d]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.mode")}</div><div class="group-row-sub">{$t("tools.cliporg.mode_hint")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={mode === "scan"} type="button" onclick={() => (mode = "scan")}>{$t("tools.cliporg.mode_scan")}</button>
            <button class="segmented-btn" class:active={mode === "copy"} type="button" onclick={() => (mode = "copy")}>{$t("tools.cliporg.mode_copy")}</button>
            <button class="segmented-btn" class:active={mode === "move"} type="button" onclick={() => (mode = "move")}>{$t("tools.cliporg.mode_move")}</button>
          </div>
        </div>
      </div>
      {#if organizing}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.cliporg.dest")}</div>
            <div class="group-row-sub mono">{dest || $t("tools.cliporg.dest_hint")}</div>
          </div>
          <div class="group-row-trailing">
            <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.layout")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={layout}>
              <option value="game">{$t("tools.cliporg.layout_game")}</option>
              <option value="game-year">{$t("tools.cliporg.layout_game_year")}</option>
              <option value="year-month">{$t("tools.cliporg.layout_year_month")}</option>
              <option value="flat">{$t("tools.cliporg.layout_flat")}</option>
            </select>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.dry_run")}</div><div class="group-row-sub">{$t("tools.cliporg.dry_run_hint")}</div></div>
          <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={dryRun} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.dedupe")}</div><div class="group-row-sub">{$t("tools.cliporg.dedupe_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={dedupe} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.recompress")}</div><div class="group-row-sub">{$t("tools.cliporg.recompress_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="50" bind:value={recompressOver} style:width="6em" /><span class="dim">MB</span>
        </div>
      </div>
      {#if recompressOver > 0}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.cliporg.target")}</div></div>
          <div class="group-row-trailing btn-row">
            <input class="input" type="number" min="1" step="5" bind:value={targetMb} style:width="6em" /><span class="dim">MB</span>
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.cliporg.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.found} {$t("tools.cliporg.found")} · {fmtBytes(result.bytes_total)}</div>
            <div class="group-row-sub">{result.imported} {$t("tools.cliporg.organized")} · {result.skipped} {$t("tools.cliporg.duplicates")} · {result.failed} {$t("tools.common.failed")}</div>
          </div>
          <div class="group-row-trailing">
            {#if dest}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(dest)}>{$t("tools.common.reveal")}</button>{/if}
          </div>
        </div>
        {#if result.recompressed.length}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{result.recompressed.filter((c) => c.ok).length} {$t("tools.cliporg.recompressed")}</div>
              <div class="group-row-sub">{fmtBytes(saved)} {$t("tools.cliporg.saved")}</div>
            </div>
          </div>
        {/if}
      </div>
    </section>

    {#if result.games.length}
      <section>
        <span class="group-label">{$t("tools.cliporg.games")}</span>
        <div class="group">
          {#each result.games as g (g.game)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{g.game}</div>
                <div class="group-row-sub">{g.clips} {$t("tools.cliporg.clips")} · {g.images} {$t("tools.cliporg.images")} · {fmtBytes(g.bytes)}</div>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.largest.length}
      <section>
        <span class="group-label">{$t("tools.cliporg.largest")}</span>
        <div class="group">
          {#each result.largest as b (b.path)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{fmtBytes(b.bytes)} <span class="dim">{b.game}</span></div>
                <div class="group-row-sub mono">{baseName(b.path)}</div>
              </div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(b.path)}>↗</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if shown.length}
      <section>
        <span class="group-label">{$t("tools.cliporg.files")}</span>
        <div class="group">
          {#each shown as it (it.source)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{it.game} <span class="dim">{it.taken_at}</span></div>
                <div class="group-row-sub mono">{it.dest ? baseName(it.dest) : baseName(it.source)}</div>
                <div class="group-row-sub">
                  {fmtBytes(it.bytes)}
                  {#if it.extra}<span class="tag">{it.extra}</span>{/if}
                  {#if it.reason === "duplicate"}<span class="tag">{$t("tools.cliporg.duplicate")}</span>{/if}
                  {#if it.status === "error"}<span class="tag">{it.reason}</span>{/if}
                </div>
              </div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(it.dest ?? it.source)}>↗</button></div>
            </div>
          {/each}
        </div>
      </section>
    {:else}
      <section><div class="group"><div class="group-row"><div class="group-row-sub">{$t("tools.cliporg.empty")}</div></div></div></section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
