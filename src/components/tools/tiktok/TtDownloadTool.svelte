<script lang="ts">
  /** Vídeo do TikTok sem marca d'água, em lote: lista colada, .txt ou perfil inteiro. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Item = { url: string; id: string; author: string; status: string; reason: string; files: string[] };
  type Result = {
    items: Item[]; ok: number; failed: number; skipped: number;
    dest: string; used_session: boolean; no_watermark: boolean;
  };

  let urls = $state("");
  let listFile = $state("");
  let profile = $state("");
  let dest = $state("");
  let watermark = $state(false);
  let quality = $state("best");
  let limit = $state(50);
  let skipExisting = $state(true);
  let writeInfo = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(!!dest && (urls.trim().length > 0 || !!listFile || profile.trim().length > 0));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tt-download") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_tt_download", {
        opts: {
          urls,
          list_file: listFile || null,
          profile: profile.trim() || null,
          dest,
          watermark,
          quality,
          limit: Number(limit) || 0,
          skip_existing: skipExisting,
          write_info: writeInfo,
          delay_ms: 1200,
          cookies: null,
          account_slug: null,
        },
      });
      showToast("success", `${result.ok} ${$t("tools.ttdl.done")}`);
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
      <div class="group-row col">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.ttdl.links")}</div>
          <div class="group-row-sub">{$t("tools.ttdl.links_hint")}</div>
        </div>
        <textarea class="input area mono" rows="5" bind:value={urls} placeholder={$t("tools.ttdl.links_ph")} spellcheck="false"></textarea>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.ttdl.list_file")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub mono">{listFile ? baseName(listFile) : $t("tools.ttdl.list_file_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if listFile}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (listFile = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(); if (f) listFile = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.ttdl.profile")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub">{$t("tools.ttdl.profile_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder="@usuario" bind:value={profile} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttdl.watermark")}</div><div class="group-row-sub">{$t("tools.ttdl.watermark_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={watermark} type="button" role="switch" aria-checked={watermark} aria-label={$t("tools.ttdl.watermark")} onclick={() => (watermark = !watermark)}><span class="toggle-knob"></span></button></div>
      </div>
      {#if !watermark}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.ttdl.quality")}</div><div class="group-row-sub">{$t("tools.ttdl.quality_hint")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={quality}>
              <option value="best">{$t("tools.ttdl.quality_best")}</option>
              <option value="h264">{$t("tools.ttdl.quality_h264")}</option>
            </select>
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttdl.limit")}</div><div class="group-row-sub">{$t("tools.ttdl.limit_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" step="10" bind:value={limit} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttdl.skip")}</div><div class="group-row-sub">{$t("tools.ttdl.skip_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={skipExisting} type="button" role="switch" aria-checked={skipExisting} aria-label={$t("tools.ttdl.skip")} onclick={() => (skipExisting = !skipExisting)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttdl.write_info")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={writeInfo} type="button" role="switch" aria-checked={writeInfo} aria-label={$t("tools.ttdl.write_info")} onclick={() => (writeInfo = !writeInfo)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ttdl.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.ok} {$t("tools.ttdl.done")}</div>
            <div class="group-row-sub">
              <span class="tag tag-success">{result.no_watermark ? $t("tools.ttdl.tag_clean") : $t("tools.ttdl.tag_watermark")}</span>
              <span class="tag">{result.used_session ? $t("tools.tt.session_on") : $t("tools.tt.session_off")}</span>
              {result.skipped} {$t("tools.ttdl.skipped")} · {result.failed} {$t("tools.ttdl.failed")}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.items as item (item.url)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">
                <span class="tag" class:tag-success={item.status === "ok"}>{$t(`tools.ttdl.status_${item.status}`)}</span>
                {item.author ? `@${item.author}` : item.id}
              </div>
              <div class="group-row-sub mono">{item.files.length ? baseName(item.files[0]) : item.reason || item.url}</div>
            </div>
            <div class="group-row-trailing btn-row">
              {#if item.files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.files[0])}>{$t("tools.common.reveal")}</button>{/if}
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(item.url)}>↗</button>
            </div>
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
  .group-row.col { flex-direction: column; align-items: stretch; gap: var(--space-2); }
  textarea.area { width: 100%; resize: vertical; }
</style>
