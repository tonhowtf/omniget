<script lang="ts">
  /** Playlist exportada -> .m3u8/.pls apontando para os arquivos do disco. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Track = { artist: string; title: string; album: string; duration_secs: number | null };
  type Matched = Track & { path: string; score: number; copied_to: string | null };
  type Result = {
    total: number;
    scanned_files: number;
    matched: Matched[];
    missing: Track[];
    m3u_path: string;
    pls_path: string | null;
    copied: number;
  };

  let source = $state<string | null>(null);
  let musicDirs = $state<string[]>([]);
  let outDir = $state<string | null>(null);
  let name = $state("playlist");
  let relative = $state(false);
  let writePls = $state(true);
  let threshold = $state(82);
  let copyTo = $state<string | null>(null);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(!!source && musicDirs.length > 0 && !!outDir);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "music-playlist") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_music_playlist", {
        opts: {
          source,
          music_dirs: musicDirs,
          out_dir: outDir,
          name: name.trim() || "playlist",
          relative,
          write_pls: writePls,
          threshold,
          copy_to: copyTo,
        },
      });
      showToast("success", `${result.matched.length}/${result.total}`);
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
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.source")}</div>
          <div class="group-row-sub mono">{source ? baseName(source) : $t("tools.m3u.source_hint")}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "Playlist", extensions: ["csv", "json", "txt", "m3u", "m3u8"] }]); if (f) source = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.folders")}</div>
          <div class="group-row-sub mono">{musicDirs.join(" · ") || "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if musicDirs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (musicDirs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) musicDirs = [...musicDirs, d]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.out_dir")}</div>
          <div class="group-row-sub mono">{outDir ?? "—"}</div>
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.m3u.name")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={name} style:width="14em" /></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.relative")}</div>
          <div class="group-row-sub">{$t("tools.m3u.relative_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={relative} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.m3u.pls")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={writePls} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.threshold")} {threshold}</div>
          <div class="group-row-sub">{$t("tools.m3u.threshold_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="range" min="60" max="100" step="1" bind:value={threshold} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.m3u.copy_to")}</div>
          <div class="group-row-sub mono">{copyTo ?? $t("tools.common.optional")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if copyTo}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (copyTo = null)}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) copyTo = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.m3u.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.matched.length}/{result.total} {$t("tools.m3u.matched")}</div>
            <div class="group-row-sub">{result.scanned_files} {$t("tools.m3u.scanned")}{#if result.copied} · {result.copied} {$t("tools.m3u.copied")}{/if}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.m3u_path)}>{$t("tools.common.reveal")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub mono">{result.m3u_path}{#if result.pls_path}<br />{result.pls_path}{/if}</div></div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.missing.length} {$t("tools.m3u.missing")}</div>
            <div class="group-row-sub">{result.missing.length ? $t("tools.m3u.missing_hint") : $t("tools.m3u.no_missing")}</div>
          </div>
        </div>
        {#each result.missing.slice(0, 200) as m, i (i)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{m.artist ? `${m.artist} — ` : ""}{m.title}</div></div>
          </div>
        {/each}
      </div>
    </section>

    {#if result.matched.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.m3u.matched")}</div></div></div>
          {#each result.matched.slice(0, 200) as m (m.path)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-sub">{m.artist ? `${m.artist} — ` : ""}{m.title} {#if m.score < 100}<span class="tag">{m.score}</span>{:else}<span class="tag tag-success">{$t("tools.m3u.exact")}</span>{/if}</div>
                <div class="group-row-sub mono">{m.path}</div>
              </div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(m.path)}>↗</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
