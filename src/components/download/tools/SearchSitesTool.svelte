<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Hit = { title: string; url: string; author: string | null; duration: number | null; thumbnail: string | null };
  type SiteResults = { site: string; hits: Hit[]; error: string | null };

  const SITES = ["youtube", "youtube_music", "soundcloud", "bilibili", "niconico", "archive"] as const;

  let query = $state("");
  let picked = $state<string[]>([...SITES]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let results = $state<SiteResults[] | null>(null);
  let queued = $state<Record<string, boolean>>({});

  const total = $derived(results ? results.reduce((n, r) => n + r.hits.length, 0) : 0);

  function toggle(site: string) {
    picked = picked.includes(site) ? picked.filter((s) => s !== site) : [...picked, site];
  }

  async function search() {
    const q = query.trim();
    if (!q || loading || picked.length === 0) return;
    loading = true;
    error = null;
    results = null;
    queued = {};
    try {
      results = await invoke<SiteResults[]>("search_sites", { query: q, sites: picked, perSite: 5 });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("downloads.tools.search_sites.error");
    } finally {
      loading = false;
    }
  }

  function duration(s: number | null): string {
    if (!s || s <= 0) return "";
    const total = Math.round(s);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const sec = String(total % 60).padStart(2, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
  }

  async function download(hit: Hit, audio: boolean) {
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.search_sites.no_folder"));
      return;
    }
    try {
      await invoke("download_from_url", {
        url: hit.url,
        outputDir,
        downloadMode: audio ? "audio" : null,
        quality: null,
        formatId: null,
        referer: null,
        cookieSlug: null,
      });
      queued = { ...queued, [hit.url]: true };
      showToast("success", $t("downloads.tools.search_sites.queued"));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      showToast("error", msg ? translateBackendError(msg, $t) : $t("common.error"));
    }
  }
</script>

<div class="ss" data-agent-readiness={loading ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.search_sites.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      search();
    }}
  >
    <input
      type="search"
      bind:value={query}
      placeholder={$t("downloads.tools.search_sites.placeholder")}
      aria-label={$t("downloads.tools.search_sites.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={loading || !query.trim() || picked.length === 0}>
      {loading ? $t("downloads.tools.search_sites.searching") : $t("downloads.tools.search_sites.search")}
    </button>
  </form>

  <div class="filters" role="group" aria-label={$t("downloads.tools.search_sites.sites")}>
    {#each SITES as site (site)}
      <button class="chip" class:on={picked.includes(site)} aria-pressed={picked.includes(site)} onclick={() => toggle(site)}>
        {$t(`downloads.tools.search_sites.site_${site}`)}
      </button>
    {/each}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if results}
    {#if total === 0 && results.every((r) => !r.error)}
      <p class="dim">{$t("downloads.tools.search_sites.none")}</p>
    {/if}
    {#each results as group (group.site)}
      <section class="group" data-site={group.site}>
        <h3>{$t(`downloads.tools.search_sites.site_${group.site}`)}</h3>
        {#if group.error}
          <p class="dim small">{$t("downloads.tools.search_sites.site_failed")}</p>
        {:else if group.hits.length === 0}
          <p class="dim small">{$t("downloads.tools.search_sites.none")}</p>
        {:else}
          <ul class="list">
            {#each group.hits as hit (hit.url)}
              <li class="hit">
                {#if hit.thumbnail}
                  <img src={hit.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" />
                {/if}
                <div class="info">
                  <span class="title" title={hit.title}>{hit.title}</span>
                  <span class="dim small">
                    {[hit.author, duration(hit.duration)].filter(Boolean).join(" · ")}
                  </span>
                </div>
                <div class="actions">
                  {#if queued[hit.url]}
                    <span class="dim small">{$t("downloads.tools.search_sites.added")}</span>
                  {:else}
                    <button class="link" onclick={() => download(hit, true)}>{$t("downloads.tools.search_sites.audio")}</button>
                    <button class="button small-btn" onclick={() => download(hit, false)}>
                      {$t("downloads.tools.search_sites.download")}
                    </button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  {/if}
</div>

<style>
  .ss {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .filters {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .chip {
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .group h3 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .hit {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .hit img {
    width: 64px;
    height: 36px;
    object-fit: cover;
    border-radius: var(--radius-sm, 4px);
    flex-shrink: 0;
    background: var(--fill-1);
  }
  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .title {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
  .small-btn {
    font-size: var(--text-sm);
  }
</style>
