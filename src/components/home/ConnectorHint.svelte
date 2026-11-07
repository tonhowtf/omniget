<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { errorKey, looksLikeUrl, type UrlLookup } from "$lib/global/connector-market";

  let { url }: { url: string } = $props();

  let hit = $state<UrlLookup | null>(null);
  let hitFor = $state("");
  let installing = $state(false);
  let failure = $state("");
  let serial = 0;

  let installedName = $derived(hit?.installed?.name ?? "");
  let offer = $derived(!hit?.installed ? hit?.catalog ?? null : null);
  let needsUpdate = $derived(!!hit?.installed && hit?.catalog?.state === "update_available");

  $effect(() => {
    const value = url.trim();
    const mine = ++serial;
    failure = "";
    if (!looksLikeUrl(value)) {
      hit = null;
      hitFor = "";
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const r = await invoke<UrlLookup>("gd_connector_for_url", { url: value });
        if (mine === serial) {
          hit = r;
          hitFor = value;
        }
      } catch {
        if (mine === serial) hit = null;
      }
    }, 300);
    return () => clearTimeout(timer);
  });

  function openDownloader() {
    goto(`/global?url=${encodeURIComponent(hitFor)}`);
  }

  async function installAndContinue() {
    if (!offer) return;
    installing = true;
    failure = "";
    try {
      await invoke("gd_market_install", { id: offer.id });
      openDownloader();
    } catch (e) {
      failure = typeof e === "string" ? e : String(e);
    } finally {
      installing = false;
    }
  }
</script>

{#if hit?.installed}
  <div class="hint" role="status">
    <p>{$t("global_dl.market.hint_installed", { name: installedName })}{needsUpdate ? ` ${$t("global_dl.market.hint_update")}` : ""}</p>
    <button type="button" class="btn btn-primary btn-sm" onclick={openDownloader}>{$t("global_dl.market.hint_open")}</button>
  </div>
{:else if offer}
  <div class="hint" role="status">
    <p>{$t("global_dl.market.hint_available", { name: offer.name })}</p>
    {#if !offer.compatible}
      <p class="sub">{$t("global_dl.market.needs_version", { version: offer.min_omniget })}</p>
    {:else}
      <button type="button" class="btn btn-primary btn-sm" onclick={installAndContinue} disabled={installing}>
        {#if installing}<span class="spinner" aria-hidden="true"></span>{/if}
        {installing ? $t("global_dl.market.hint_installing") : $t("global_dl.market.hint_install")}
      </button>
    {/if}
    {#if failure}
      <p class="sub" role="alert">{$t(errorKey(failure))}</p>
    {/if}
  </div>
{/if}

<style>
  .hint {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    background: var(--surface);
    font-size: var(--text-sm);
    text-align: center;
  }
  p {
    margin: 0;
  }
  .sub {
    flex-basis: 100%;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
</style>
