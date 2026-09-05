<script lang="ts">
  /**
   * Achar a fonte (estudo 67): link de destino, criador, checagem do link,
   * cópia na Wayback Machine e busca reversa (Google Lens, TinEye, Yandex,
   * Bing, SauceNAO). Também expande pin.it.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, openUrl } from "$lib/tools/rt";
  import { fmtCount, kindKey, loadCookies, looksAi, type SourceOut } from "$lib/tools/pinterest";

  let url = $state("");
  let cookies = $state(loadCookies());
  let busy = $state(false);
  let out = $state<SourceOut | null>(null);

  async function run() {
    if (!url.trim() || busy) return;
    busy = true; out = null;
    try {
      out = await invoke<SourceOut>("tool_pin_source", { url, cookies: cookies || null });
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function copy(text: string) { await navigator.clipboard.writeText(text); showToast("success", $t("tools.common.copied") as string); }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={url} placeholder={$t("tools.pinterest.pin_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.pinterest.inspect")}</button></div>
      </div>
    </div>
  </section>

  {#if out}
    {@const p = out.pin}
    <section>
      <div class="group">
        <div class="group-row hero">
          <div class="hero-img" style:background={p.dominant_color ?? "var(--fill-1)"}>{#if p.image_large?.url ?? p.thumb}<img src={p.image_large?.url ?? p.thumb} alt={p.title} />{/if}</div>
          <div class="group-row-content">
            <div class="group-row-title strong">{p.title || p.alt_text || p.id}</div>
            <div class="tags">
              <span class="tag">{$t(kindKey(p))}</span>
              {#if p.image?.width}<span class="tag">{p.image.width}×{p.image.height}</span>{/if}
              {#if p.is_promoted}<span class="tag tag-warning">{$t("tools.pinterest.promoted")}</span>{/if}
              {#if looksAi(p)}<span class="tag tag-accent">{$t("tools.pinterest.ai")} · {p.ai.labeled ? $t("tools.pinterest.ai_labeled_by_pinterest") : p.ai.keyword}</span>{/if}
              <span class="tag">{fmtCount(p.saves)} {$t("tools.pinterest.saves")}</span>
            </div>
            {#if out.resolved_url !== url.trim()}<div class="group-row-sub">{$t("tools.pinterest.expanded")}: <span class="mono">{out.resolved_url}</span></div>{/if}
            {#if p.created_at}<div class="group-row-sub">{$t("tools.pinterest.created")}: {p.created_at}</div>{/if}
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(p.url)}>↗</button></div>
        </div>
      </div>
    </section>

    <section>
      <span class="group-label">{$t("tools.pinterest.link")}</span>
      <div class="group">
        {#if out.link}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title mono">{out.link.url}</div>
              <div class="group-row-sub">
                {#if out.link.ok}<span class="tag tag-success">{$t("tools.pinterest.link_alive")}</span>{:else}<span class="tag tag-danger">{$t("tools.pinterest.link_dead")}</span>{/if}
                {#if out.link.status} · {$t("tools.pinterest.link_status")} {out.link.status}{/if}
                {#if out.link.final_url && out.link.final_url !== out.link.url} · {$t("tools.pinterest.final_url")}: <span class="mono">{out.link.final_url}</span>{/if}
                {#if out.link.error} · {out.link.error}{/if}
              </div>
            </div>
            <div class="group-row-trailing btn-row"><button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(out!.link!.url)}>{$t("tools.pinterest.open_link")}</button></div>
          </div>
          {#if !out.link.ok}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.wayback")}</div><div class="group-row-sub mono">{out.wayback ?? $t("tools.pinterest.no_wayback")}</div></div>
              <div class="group-row-trailing">{#if out.wayback}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(out!.wayback!)}>{$t("tools.common.open")}</button>{/if}</div>
            </div>
          {/if}
        {:else}
          <div class="group-row"><div class="group-row-sub">{$t("tools.pinterest.uploaded")}{#if p.domain} · {p.domain}{/if}</div></div>
        {/if}
        {#if p.rich}
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.rich")}</div><div class="group-row-sub">{[p.rich.site_name, p.rich.title, p.rich.description].filter(Boolean).join(" · ")}</div></div></div>
        {/if}
        {#if p.attribution}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.attribution")}</div><div class="group-row-sub">{[p.attribution.author_name, p.attribution.provider_name, p.attribution.title].filter(Boolean).join(" · ")}</div></div>
            <div class="group-row-trailing">{#if p.attribution.author_url ?? p.attribution.url}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl((p.attribution!.author_url ?? p.attribution!.url)!)}>↗</button>{/if}</div>
          </div>
        {/if}
        {#if p.creator?.username}
          <div class="group-row">
            {#if p.creator.avatar}<img class="avatar" src={p.creator.avatar} alt="" />{/if}
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.creator")}: {p.creator.name ?? p.creator.username}</div><div class="group-row-sub">@{p.creator.username}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(`https://www.pinterest.com/${p.creator!.username}/`)}>↗</button></div>
          </div>
        {/if}
        {#if p.pinner?.username && p.pinner.username !== p.creator?.username}
          <div class="group-row">
            {#if p.pinner.avatar}<img class="avatar" src={p.pinner.avatar} alt="" />{/if}
            <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.pinner")}: {p.pinner.name ?? p.pinner.username}</div><div class="group-row-sub">@{p.pinner.username}{#if p.board?.name} · {p.board.name}{/if}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(`https://www.pinterest.com/${p.pinner!.username}/`)}>↗</button></div>
          </div>
        {/if}
      </div>
    </section>

    {#if out.reverse.length}
      <section>
        <span class="group-label">{$t("tools.pinterest.reverse")}</span>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content"><div class="chips">{#each out.reverse as [name, href] (name)}<button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl(href)}>{name}</button>{/each}</div></div>
            <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy((p.image ?? p.image_large)!.url)}>{$t("tools.pinterest.copy_url")}</button></div>
          </div>
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .hero { align-items: flex-start; }
  .hero-img { flex: 0 0 140px; border-radius: var(--radius-md); overflow: hidden; }
  .hero-img img { display: block; width: 100%; height: auto; }
  .strong { font-weight: 600; }
  .tags, .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .avatar { width: 36px; height: 36px; border-radius: 50%; object-fit: cover; flex-shrink: 0; }
</style>
