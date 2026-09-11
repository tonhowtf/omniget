<script lang="ts">
  /** Fotos duplicadas e parecidas (dHash), reusando o motor do Pinterest. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type File = { path: string; bytes: number; width: number; height: number; best: boolean };
  type Group = { kind: string; distance: number; files: File[]; wasted_bytes: number };
  type Result = { scanned: number; hashed: number; groups: Group[]; wasted_bytes: number };

  let dirs = $state<string[]>([]);
  let threshold = $state(5);
  let minSize = $state(16);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const KIND: Record<string, string> = { exact: "tools.imgdupes.kind_exact", same: "tools.imgdupes.kind_same", near: "tools.imgdupes.kind_near" };

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-dupes") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!dirs.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_img_dupes", {
        opts: { dirs, threshold, min_size: minSize * 1024, skip_hidden: true },
      });
      showToast("success", `${result.groups.length} ${$t("tools.imgdupes.groups")}`);
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgdupes.folders")}</div><div class="group-row-sub mono">{dirs.join(" · ") || "—"}</div></div>
        <div class="group-row-trailing btn-row">{#if dirs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dirs = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dirs = [...dirs, d]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgdupes.threshold")} {threshold === 0 ? $t("tools.imgdupes.exact_only") : threshold}</div><div class="group-row-sub">{$t("tools.imgdupes.note")}</div></div>
        <div class="group-row-trailing"><input type="range" min="0" max="12" step="1" bind:value={threshold} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgdupes.min_size")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" step="8" bind:value={minSize} style:width="6em" /><span class="dim">KB</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !dirs.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.imgdupes.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{result.groups.length} {$t("tools.imgdupes.groups")}</div>
            <div class="group-row-sub">{result.scanned} {$t("tools.imgdupes.scanned")} · {result.hashed} {$t("tools.imgdupes.hashed")} · <strong>{fmtBytes(result.wasted_bytes)}</strong> {$t("tools.imgdupes.wasted")}</div>
          </div>
        </div>
      </div>
    </section>
    {#each result.groups.slice(0, 60) as g, i (i)}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title"><span class="tag">{$t(KIND[g.kind] ?? "tools.imgdupes.kind_near")}</span> {#if g.distance}<span class="dim">{$t("tools.imgdupes.distance")} {g.distance}</span>{/if}</div>
              <div class="group-row-sub">{fmtBytes(g.wasted_bytes)} {$t("tools.imgdupes.wasted")}</div></div>
          </div>
          {#each g.files as f (f.path)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-sub mono">{f.path}</div>
                <div class="group-row-sub">{f.width}×{f.height} · {fmtBytes(f.bytes)} {#if f.best}<span class="tag tag-success">{$t("tools.imgdupes.keep")}</span>{/if}</div>
              </div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f.path)}>↗</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
