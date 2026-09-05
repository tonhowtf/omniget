<script lang="ts">
  /**
   * Baixar pin (estudo 67): inspeciona o pin (original, vídeo, carrossel,
   * story, estatísticas, sinais de anúncio/IA) e baixa na melhor qualidade,
   * com WebP → PNG para quem não abre WebP.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, openUrl, pickDir, reveal } from "$lib/tools/rt";
  import { defaultDownload, fmtCount, fmtDuration, kindKey, loadCookies, looksAi, type Inspect, type Pin, type PinFiles } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";
  import PinDownloadOptions from "./PinDownloadOptions.svelte";

  let url = $state("");
  let cookies = $state(loadCookies());
  let busy = $state<string | null>(null);
  let info = $state<Inspect | null>(null);
  let opts = $state(defaultDownload());
  let result = $state<PinFiles | null>(null);
  let showCookies = $state(false);

  let pin = $derived(info?.pin ?? null);

  async function inspect() {
    if (!url.trim() || busy) return;
    busy = "inspect"; info = null; result = null;
    try {
      info = await invoke<Inspect>("tool_pin_inspect", { url, cookies: cookies || null });
      if (!info.pin) showToast("info", $t("tools.pinterest.need_pin") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  async function download() {
    if (!pin || busy) return;
    if (!opts.dest) { const d = await pickDir(); if (!d) return; opts.dest = d; }
    busy = "download"; result = null;
    try {
      result = await invoke<PinFiles>("tool_pin_download", { url: info?.resolved_url ?? url, opts: { ...opts, skip_downloaded: false, section_folders: false }, cookies: cookies || null });
      showToast("success", `${result.files.length} ${$t("tools.common.files")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  function best(p: Pin): string {
    return p.image?.url ?? p.image_large?.url ?? p.thumb ?? "";
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={$t("tools.pinterest.pin_placeholder")} onkeydown={(e) => e.key === "Enter" && inspect()} /></div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showCookies = !showCookies)} title={$t("tools.pinterest.cookies_hint")}>🍪</button>
          <button class="btn btn-primary" type="button" disabled={busy !== null || !url.trim()} onclick={inspect}>{busy === "inspect" ? $t("tools.common.working") : $t("tools.pinterest.inspect")}</button>
        </div>
      </div>
      {#if showCookies}<PinCookies bind:value={cookies} />{/if}
    </div>
  </section>

  {#if pin}
    <section>
      <div class="group">
        <div class="group-row hero">
          <div class="hero-img" style:background={pin.dominant_color ?? "var(--fill-1)"}>
            {#if best(pin)}<img src={pin.image_large?.url ?? best(pin)} alt={pin.title} />{/if}
          </div>
          <div class="group-row-content">
            <div class="group-row-title strong">{pin.title || pin.alt_text || pin.id}</div>
            {#if pin.description}<div class="group-row-sub clamp">{pin.description}</div>{/if}
            <div class="tags">
              <span class="tag">{$t(kindKey(pin))}</span>
              {#if pin.image?.width}<span class="tag">{pin.image.width}×{pin.image.height}</span>{/if}
              {#if pin.video}<span class="tag">{pin.video.width}×{pin.video.height} · {fmtDuration(pin.video.duration_ms)}{#if !pin.video.mp4} · HLS{/if}</span>{/if}
              {#if pin.extras.length}<span class="tag">{pin.extras.length} ×</span>{/if}
              {#if pin.is_promoted}<span class="tag tag-warning">{$t("tools.pinterest.promoted")}</span>{/if}
              {#if looksAi(pin)}<span class="tag tag-accent">{$t("tools.pinterest.ai")} · {pin.ai.labeled ? $t("tools.pinterest.ai_labeled_by_pinterest") : pin.ai.keyword}</span>{/if}
            </div>
            <div class="stats">
              <span><b>{fmtCount(pin.saves)}</b> {$t("tools.pinterest.saves")}</span>
              <span><b>{fmtCount(pin.repins)}</b> {$t("tools.pinterest.repins")}</span>
              <span><b>{fmtCount(pin.comments)}</b> {$t("tools.pinterest.comments")}</span>
              <span><b>{fmtCount(pin.reactions)}</b> {$t("tools.pinterest.reactions")}</span>
            </div>
            <div class="group-row-sub">
              {#if pin.pinner?.username}{$t("tools.pinterest.pinner")} <b>{pin.pinner.name ?? pin.pinner.username}</b>{/if}
              {#if pin.board?.name} · {$t("tools.pinterest.board")} <b>{pin.board.name}</b>{/if}
              {#if pin.created_at} · {pin.created_at}{/if}
            </div>
            <div class="btn-row">
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(pin!.url)}>{$t("tools.pinterest.open_pinterest")}</button>
              {#if pin.link}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(pin!.link!)}>{$t("tools.pinterest.open_link")} · {pin.domain}</button>{/if}
              {#if best(pin)}<button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(best(pin!))}>{$t("tools.pinterest.copy_url")}</button>{/if}
            </div>
          </div>
        </div>
        {#if pin.extras.length}
          <div class="group-row">
            <div class="group-row-content">
              <div class="strip">
                {#each pin.extras as ex (ex.index)}
                  <div class="strip-item">
                    {#if ex.kind === "video"}<div class="strip-video">▶</div>{:else if ex.image}<img src={ex.image.url} alt="" loading="lazy" />{/if}
                  </div>
                {/each}
              </div>
            </div>
          </div>
        {/if}
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.pinterest.options")}</span>
      <div class="group">
        <PinDownloadOptions bind:opts sections={false} sync={false} />
        <div class="group-row">
          <div class="group-row-content">{#if result}<div class="group-row-sub mono">{result.files.join("\n")}</div>{/if}</div>
          <div class="group-row-trailing btn-row">
            {#if result?.files.length}<button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.files[0])}>{$t("tools.common.reveal")}</button>{/if}
            <button class="btn btn-primary" type="button" disabled={busy !== null} onclick={download}>{busy === "download" ? $t("tools.common.downloading") : $t("tools.pinterest.download")}</button>
          </div>
        </div>
      </div>
    </section>
  {:else if info && !info.pin}
    <section><div class="group"><div class="group-row"><div class="group-row-sub">{$t("tools.pinterest.need_pin")}</div></div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; white-space: pre-wrap; }
  .hero { align-items: flex-start; }
  .hero-img { flex: 0 0 200px; max-width: 40%; border-radius: var(--radius-md); overflow: hidden; }
  .hero-img img { display: block; width: 100%; height: auto; }
  .strong { font-weight: 600; font-size: var(--text-base); }
  .clamp { display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .tags { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-top: 2px; }
  .stats { display: flex; flex-wrap: wrap; gap: var(--space-3); font-size: var(--text-sm); color: var(--text-muted); }
  .stats b { color: var(--text); }
  .strip { display: flex; gap: var(--space-2); overflow-x: auto; padding-bottom: 4px; }
  .strip-item { flex: 0 0 90px; height: 120px; border-radius: var(--radius-md); overflow: hidden; background: var(--fill-1); }
  .strip-item img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .strip-video { display: flex; align-items: center; justify-content: center; height: 100%; font-size: 22px; color: var(--text-dim); }
</style>
