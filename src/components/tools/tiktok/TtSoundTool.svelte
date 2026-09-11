<script lang="ts">
  /** Só o áudio do TikTok — o "som" — com o crédito nas tags e num sidecar de texto. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Attribution = {
    sound: string; sound_author: string; sound_url: string; sound_id: string; original: boolean;
    video_author: string; video_title: string; video_url: string; video_id: string; created: string; duration: number;
  };
  type Item = { url: string; status: string; reason: string; file: string; sidecar: string; tagged: boolean; attribution: Attribution };
  type Result = { items: Item[]; ok: number; failed: number; skipped: number; dest: string; used_session: boolean };

  let urls = $state("");
  let listFile = $state("");
  let dest = $state("");
  let format = $state("mp3");
  let tagFile = $state(true);
  let writeSidecar = $state(true);
  let skipExisting = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(!!dest && (urls.trim().length > 0 || !!listFile));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "tt-sound") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_tt_sound", {
        opts: {
          urls,
          list_file: listFile || null,
          dest,
          format,
          tag_file: tagFile,
          write_sidecar: writeSidecar,
          skip_existing: skipExisting,
          delay_ms: 1200,
          cookies: null,
          account_slug: null,
        },
      });
      showToast("success", `${result.ok} ${$t("tools.ttsound.done")}`);
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
          <div class="group-row-title">{$t("tools.ttsound.links")}</div>
          <div class="group-row-sub">{$t("tools.ttsound.links_hint")}</div>
        </div>
        <textarea class="input area mono" rows="5" bind:value={urls} placeholder={$t("tools.ttsound.links_ph")} spellcheck="false"></textarea>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.ttsound.list_file")} <span class="dim">({$t("tools.common.optional")})</span></div>
          <div class="group-row-sub mono">{listFile ? baseName(listFile) : "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if listFile}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (listFile = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(); if (f) listFile = f; }}>{$t("tools.common.choose")}</button>
        </div>
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttsound.format")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={format}>
            <option value="mp3">MP3</option>
            <option value="m4a">M4A</option>
            <option value="best">{$t("tools.ttsound.format_best")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttsound.tag_file")}</div><div class="group-row-sub">{$t("tools.ttsound.tag_file_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={tagFile} type="button" role="switch" aria-checked={tagFile} aria-label={$t("tools.ttsound.tag_file")} onclick={() => (tagFile = !tagFile)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttsound.sidecar")}</div><div class="group-row-sub">{$t("tools.ttsound.sidecar_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={writeSidecar} type="button" role="switch" aria-checked={writeSidecar} aria-label={$t("tools.ttsound.sidecar")} onclick={() => (writeSidecar = !writeSidecar)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.ttsound.skip")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={skipExisting} type="button" role="switch" aria-checked={skipExisting} aria-label={$t("tools.ttsound.skip")} onclick={() => (skipExisting = !skipExisting)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.ttsound.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.ok} {$t("tools.ttsound.done")}</div>
            <div class="group-row-sub">
              <span class="tag">{result.used_session ? $t("tools.tt.session_on") : $t("tools.tt.session_off")}</span>
              {result.skipped} {$t("tools.ttsound.skipped")} · {result.failed} {$t("tools.ttsound.failed")}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>{$t("tools.common.reveal")}</button></div>
        </div>
        {#each result.items as item (item.url)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">
                <span class="tag" class:tag-success={item.status === "ok"}>{$t(`tools.ttsound.status_${item.status}`)}</span>
                {item.attribution.sound || baseName(item.file) || item.url}
              </div>
              <div class="group-row-sub">
                {#if item.attribution.sound_author}{$t("tools.ttsound.by")} {item.attribution.sound_author}{/if}
                {#if item.attribution.original} · {$t("tools.ttsound.original")}{/if}
                {#if item.tagged} · <span class="tag tag-success">{$t("tools.ttsound.tagged")}</span>{/if}
              </div>
              {#if item.reason}<div class="group-row-sub">{item.reason}</div>{/if}
            </div>
            <div class="group-row-trailing btn-row">
              {#if item.attribution.sound_url}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(item.attribution.sound_url)}>{$t("tools.ttsound.open_sound")}</button>{/if}
              {#if item.sidecar}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.sidecar)}>{$t("tools.ttsound.credit")}</button>{/if}
              {#if item.file}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.file)}>{$t("tools.common.reveal")}</button>{/if}
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
