<script lang="ts">
  /** Índice unificado dos favoritos de DeviantArt, ArtStation e Flickr, com flag de duplicado. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Row = {
    platform: string; id: string; title: string; author: string; date: string;
    tags: string[]; page_url: string; media_url: string; preview_url: string;
    width: number; height: number; dupe: string; local_file: string; distance: number | null;
  };
  type Result = {
    total: number; by_platform: [string, number][]; dupes: number; checked: number;
    used_session: boolean; csv_path: string | null; json_path: string | null;
    out_dir: string; sample: Row[];
  };

  let deviantart = $state("");
  let artstation = $state("");
  let flickr = $state("");
  let outDir = $state("");
  let localDir = $state("");
  let checkDupes = $state(false);
  let threshold = $state(5);
  let limit = $state(300);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(!!outDir && (deviantart.trim() || artstation.trim() || flickr.trim()).length > 0);
  const files = $derived([result?.csv_path, result?.json_path].filter((f): f is string => !!f));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "art-favorites-index") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_art_favorites", {
        opts: {
          deviantart: deviantart.trim() || null,
          artstation: artstation.trim() || null,
          flickr: flickr.trim() || null,
          out_dir: outDir,
          local_dir: localDir || null,
          limit: Number(limit) || null,
          check_dupes: checkDupes,
          threshold: Number(threshold) || 0,
          max_check: 300,
          write_csv: true,
          write_json: true,
          account_slug: null,
        },
      });
      showToast("success", `${result.total} ${$t("tools.artfav.items")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">DeviantArt</div><div class="group-row-sub">{$t("tools.artfav.user_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder="usuario" bind:value={deviantart} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">ArtStation</div><div class="group-row-sub">{$t("tools.artfav.user_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder="usuario" bind:value={artstation} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">Flickr</div><div class="group-row-sub">{$t("tools.artfav.user_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder="usuario" bind:value={flickr} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{outDir || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.artfav.local_dir")}</div>
          <div class="group-row-sub mono">{localDir || $t("tools.common.optional")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if localDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (localDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) localDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.artfav.check_dupes")}</div>
          <div class="group-row-sub">{$t("tools.artfav.check_dupes_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="toggle" class:on={checkDupes} type="button" role="switch" aria-checked={checkDupes} aria-label={$t("tools.artfav.check_dupes")} onclick={() => (checkDupes = !checkDupes)}><span class="toggle-knob"></span></button></div>
      </div>
      {#if checkDupes}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.artfav.threshold")} {threshold}</div>
            <div class="group-row-sub">{$t("tools.artfav.threshold_hint")}</div>
          </div>
          <div class="group-row-trailing"><input type="range" min="0" max="12" step="1" bind:value={threshold} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.artfav.limit")}</div>
          <div class="group-row-sub">{$t("tools.artfav.limit_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="100" bind:value={limit} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {:else}
            <div class="group-row-sub">{$t("tools.artfav.session_hint")}</div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.artfav.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.total} {$t("tools.artfav.items")}</div>
            <div class="group-row-sub">
              <span class="tag" class:tag-success={result.used_session}>{result.used_session ? $t("tools.tumblr.session_on") : $t("tools.tumblr.session_off")}</span>
              {result.dupes} {$t("tools.artfav.dupes")}{result.checked ? ` · ${result.checked} ${$t("tools.artfav.checked")}` : ""}
            </div>
            <div class="group-row-sub">{result.by_platform.map(([p, n]) => `${p} ${n}`).join(" · ")}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.out_dir ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        {#each result.sample as r (`${r.platform}-${r.id}`)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{r.title || "—"} <span class="dim">{r.author}</span></div>
              <div class="group-row-sub">
                <span class="tag">{r.platform}</span>
                {r.width && r.height ? `${r.width}×${r.height}` : ""}
                {#if r.dupe}<span class="tag tag-success">{r.dupe === "hash" ? $t("tools.artfav.dupe_hash") : $t("tools.artfav.dupe_name")}</span>{/if}
              </div>
              {#if r.local_file}<div class="group-row-sub mono">{r.local_file}</div>{/if}
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(r.page_url)}>↗</button></div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
