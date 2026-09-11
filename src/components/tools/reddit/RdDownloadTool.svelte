<script lang="ts">
  /** Mídia de um post do Reddit: vídeo do v.redd.it com áudio, galeria, imagem e link de fora. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Info = {
    id: string; title: string; author: string; subreddit: string; created: string;
    score: number; num_comments: number; permalink: string; over_18: boolean;
  };
  type Result = {
    used_session: boolean; kind: string; source: string; files: string[]; dest: string; info: Info; log_tail: string };

  let url = $state("");
  let dest = $state("");
  let writeInfo = $state(true);
  let infoFormat = $state("json");
  let audioOnly = $state(false);
  let cookies = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const KIND: Record<string, string> = {
    video: "tools.rddl.kind_video",
    gallery: "tools.rddl.kind_gallery",
    image: "tools.rddl.kind_image",
    external: "tools.rddl.kind_external",
    text: "tools.rddl.kind_text",
  };

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "rd-download") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || !dest || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_rd_download", {
        opts: {
          url: url.trim(),
          dest,
          write_info: writeInfo,
          info_format: infoFormat,
          audio_only: audioOnly,
          cookies: cookies.trim() || null,
          delay_ms: 900,
        },
      });
      showToast("success", `${result.files.length} ${$t("tools.rddl.files")}`);
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
          <div class="group-row-title">{$t("tools.rddl.url")}</div>
          <div class="group-row-sub">{$t("tools.rddl.url_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" placeholder={$t("tools.rddl.url_ph")} bind:value={url} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{dest || "—"}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rddl.audio_only")}</div><div class="group-row-sub">{$t("tools.rddl.audio_only_hint")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={audioOnly} type="button" role="switch" aria-checked={audioOnly} aria-label={$t("tools.rddl.audio_only")} onclick={() => (audioOnly = !audioOnly)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rddl.write_info")}</div><div class="group-row-sub">{$t("tools.rddl.write_info_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if writeInfo}
            <select class="input" bind:value={infoFormat}>
              <option value="json">JSON</option>
              <option value="txt">TXT</option>
              <option value="both">JSON + TXT</option>
            </select>
          {/if}
          <button class="toggle" class:on={writeInfo} type="button" role="switch" aria-checked={writeInfo} aria-label={$t("tools.rddl.write_info")} onclick={() => (writeInfo = !writeInfo)}><span class="toggle-knob"></span></button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.rddl.cookies")} <span class="dim">({$t("tools.common.optional")})</span></div><div class="group-row-sub mono">{cookies ? baseName(cookies) : $t("tools.rddl.cookies_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if cookies}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (cookies = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(); if (f) cookies = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim() || !dest} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.rddl.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.info.title || $t("tools.rddl.files")}</div>
            <div class="group-row-sub">
              <span class="tag">{$t(KIND[result.kind] ?? "tools.rddl.kind_external")}</span>
              {#if result.info.subreddit}r/{result.info.subreddit} · u/{result.info.author} · {result.info.created}{/if}
            </div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if result.info.permalink}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(result?.info.permalink ?? "")}>{$t("tools.rddl.open_post")}</button>{/if}
          </div>
        </div>
        {#each result.files as f (f)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub mono">{baseName(f)}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>{$t("tools.common.reveal")}</button></div>
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
