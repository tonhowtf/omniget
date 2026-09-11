<script lang="ts">
  /** Ver e limpar EXIF/GPS. A limpeza reescreve o contêiner, sem reencodar. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Field = { tag: string; label: string; group: string; value: string; sensitive: boolean };
  type Report = {
    path: string;
    bytes: number;
    has_exif: boolean;
    fields: Field[];
    gps: [number, number] | null;
    camera: string | null;
    taken: string | null;
    software: string | null;
    sensitive_count: number;
    error: string | null;
  };
  type StripItem = { input: string; output: string | null; bytes_before: number; bytes_after: number; ok: boolean; error: string | null };
  type StripResult = { items: StripItem[] };

  let files = $state<string[]>([]);
  let reports = $state<Report[]>([]);
  let expanded = $state<string | null>(null);
  let inPlace = $state(false);
  let outDir = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<ToolProgress | null>(null);
  let stripped = $state<StripResult | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "img-exif") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function coords(gps: [number, number]): string {
    return `${gps[0].toFixed(6)}, ${gps[1].toFixed(6)}`;
  }

  async function analyze() {
    if (!files.length || busy) return;
    busy = "read";
    stripped = null;
    try {
      reports = await invoke<Report[]>("tool_exif_read", { inputs: files });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function strip() {
    if (!files.length || busy) return;
    busy = "strip";
    progress = null;
    try {
      stripped = await invoke<StripResult>("tool_exif_strip", {
        opts: { inputs: files, in_place: inPlace, output_dir: outDir, suffix: "" },
      });
      const failed = stripped.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${stripped.items.length - failed} ${$t("tools.common.done")}`);
      if (!failed) await analyze();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function copyCoords(gps: [number, number]) {
    await navigator.clipboard.writeText(coords(gps));
    showToast("success", $t("tools.common.copied") as string);
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub">{$t("tools.exif.formats")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => { files = []; reports = []; stripped = null; }}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; reports = []; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.exif.in_place")}</div><div class="group-row-sub mono">{inPlace ? "" : outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="toggle" class:on={inPlace} type="button" role="switch" aria-checked={inPlace} aria-label={$t("tools.exif.in_place")} onclick={() => (inPlace = !inPlace)}><span class="toggle-knob"></span></button>
          {#if !inPlace}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-sub">{$t("tools.exif.lossless")}</div>
          {#if busy === "strip"}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary" type="button" disabled={busy !== null || !files.length} onclick={analyze}>{busy === "read" ? $t("tools.common.working") : $t("tools.exif.analyze")}</button>
          <button class="btn btn-primary" type="button" disabled={busy !== null || !files.length} onclick={strip}>{busy === "strip" ? $t("tools.common.working") : $t("tools.exif.strip")}</button>
        </div>
      </div>
    </div>
  </section>

  {#each reports as r (r.path)}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(r.path)} {#if r.sensitive_count}<span class="tag tag-danger">{r.sensitive_count} {$t("tools.exif.sensitive")}</span>{/if}</div>
            <div class="group-row-sub">
              {#if r.error}<span class="tag tag-danger">{$t("tools.common.failed")}</span> {r.error}
              {:else if !r.has_exif}{$t("tools.exif.no_exif")} · {fmtBytes(r.bytes)}
              {:else}{r.fields.length} {$t("tools.exif.fields")} · {fmtBytes(r.bytes)}{/if}
            </div>
          </div>
          {#if r.has_exif}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => (expanded = expanded === r.path ? null : r.path)}>{expanded === r.path ? "−" : "+"}</button></div>{/if}
        </div>
        {#if r.gps}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.exif.gps")}</div><div class="group-row-sub mono">{coords(r.gps)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copyCoords(r.gps!)}>{$t("tools.exif.copy_coords")}</button></div>
          </div>
        {/if}
        {#if r.camera}<div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.exif.camera")}</div><div class="group-row-sub">{r.camera}</div></div></div>{/if}
        {#if r.taken}<div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.exif.taken")}</div><div class="group-row-sub">{r.taken}</div></div></div>{/if}
        {#if r.software}<div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.exif.software")}</div><div class="group-row-sub">{r.software}</div></div></div>{/if}
        {#if expanded === r.path}
          {#each r.fields as f (f.tag + f.group)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub"><span class="dim">{f.group}</span> {f.label || f.tag} {#if f.sensitive}<span class="tag tag-warning">{$t("tools.exif.sensitive")}</span>{/if}</div></div>
              <div class="group-row-trailing"><span class="mono">{f.value}</span></div>
            </div>
          {/each}
        {/if}
      </div>
    </section>
  {/each}

  {#if stripped}
    <section><div class="group">
      {#each stripped.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}<div class="group-row-sub">{fmtBytes(item.bytes_before)} → {fmtBytes(item.bytes_after)} <span class="dim">· {fmtBytes(item.bytes_before - item.bytes_after)} {$t("tools.exif.removed")}</span></div>
            {:else}<div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>{/if}
          </div>
          {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
