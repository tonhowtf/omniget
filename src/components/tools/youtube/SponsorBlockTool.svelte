<script lang="ts">
  /** Segmentos do SponsorBlock (estudo 43) e os argumentos do yt-dlp para baixar sem eles. */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, openUrl } from "$lib/tools/rt";

  type Segment = { uuid: string; segment: [number, number]; category: string; action_type: string; votes: number; locked: number; video_duration: number; description: string };
  type Result = { video_id: string; segments: Segment[]; skipped_seconds: number; ytdlp_args: string };

  const COLORS: Record<string, string> = { sponsor: "#00d400", selfpromo: "#ffff00", interaction: "#cc00ff", intro: "#00ffff", outro: "#0202ed", preview: "#008fd6", music_offtopic: "#ff9900", filler: "#7300ff", exclusive_access: "#008a5c", poi_highlight: "#ff1684", chapter: "#ffd679" };

  let url = $state("");
  let busy = $state(false);
  let result = $state<Result | null>(null);

  async function lookup() {
    if (!url.trim() || busy) return;
    busy = true;
    result = null;
    try {
      result = await invoke<Result>("tool_sponsorblock", { url, categories: null });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("tools.common.copied") as string);
  }

  function chapters(r: Result): string {
    return r.segments.map((s) => `${fmtSecs(s.segment[0])} ${s.category}${s.description ? ` — ${s.description}` : ""}`).join("\n");
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.common.yt_url")} onkeydown={(e) => e.key === "Enter" && lookup()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={lookup}>{busy ? $t("tools.common.working") : $t("tools.sb.lookup")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section>
      <span class="group-label">{result.segments.length} {$t("tools.sb.segments")} · {fmtSecs(result.skipped_seconds)} {$t("tools.sb.skippable")}</span>
      <div class="group">
        {#if result.segments.length}
          <div class="group-row">
            <div class="bar" title={result.video_id}>
              {#each result.segments as s (s.uuid)}
                {#if s.video_duration > 0}
                  <span class="seg" style:left="{(s.segment[0] / s.video_duration) * 100}%" style:width="{Math.max(0.4, ((s.segment[1] - s.segment[0]) / s.video_duration) * 100)}%" style:background={COLORS[s.category] ?? "#999"}></span>
                {/if}
              {/each}
            </div>
          </div>
        {/if}
        {#each result.segments as s (s.uuid)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title"><span class="dot" style:background={COLORS[s.category] ?? "#999"}></span> {s.category} <span class="dim">· {s.action_type}</span>{#if s.description} <span class="dim">· {s.description}</span>{/if}</div>
              <div class="group-row-sub">{fmtSecs(s.segment[0])} → {fmtSecs(s.segment[1])} ({fmtSecs(s.segment[1] - s.segment[0])}) · {s.votes} {$t("tools.sb.votes")}{#if s.locked} · 🔒{/if}</div>
            </div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(`https://www.youtube.com/watch?v=${result!.video_id}&t=${Math.floor(s.segment[0])}s`)}>{$t("tools.common.open")}</button></div>
          </div>
        {/each}
        {#if result.segments.length === 0}<div class="group-row"><div class="group-row-sub">{$t("tools.sb.none")}</div></div>{/if}
      </div>
    </section>
    {#if result.ytdlp_args}
      <section>
        <span class="group-label">{$t("tools.sb.download_without")}</span>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{$t("tools.sb.args_hint")}</div><div class="group-row-title mono">{result.ytdlp_args}</div></div>
            <div class="group-row-trailing btn-row">
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(result!.ytdlp_args)}>{$t("tools.common.copy")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(chapters(result!))}>{$t("tools.sb.copy_chapters")}</button>
            </div>
          </div>
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-sm); }
  .bar { position: relative; width: 100%; height: 10px; border-radius: 5px; background: var(--fill-2); overflow: hidden; }
  .seg { position: absolute; top: 0; bottom: 0; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; vertical-align: middle; }
</style>
