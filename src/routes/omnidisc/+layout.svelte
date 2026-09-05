<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { hasInstances, isMemberListOpen, initOmnidisc } from "$lib/stores/omnidisc-store.svelte";
  import { initVoice } from "$lib/stores/omnidisc-voice-store.svelte";
  import { initStream } from "$lib/stores/omnidisc-stream-store.svelte";
  import InstanceRail from "$components/omnidisc/InstanceRail.svelte";
  import ChannelList from "$components/omnidisc/ChannelList.svelte";
  import MemberList from "$components/omnidisc/MemberList.svelte";
  import StorageBanner from "$components/omnidisc/StorageBanner.svelte";
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();

  let settings = $derived(getSettings());
  let enabled = $derived(settings?.omnidisc?.enabled ?? true);
  let showShell = $derived(hasInstances());
  let inChannel = $derived(page.url.pathname.startsWith("/omnidisc/g/"));
  let isStreamPopout = $derived(page.url.pathname === "/omnidisc/stream");
  let showMembers = $derived(inChannel && isMemberListOpen());

  onMount(() => {
    void initOmnidisc();
    void initVoice();
    void initStream();
  });
</script>

{#if isStreamPopout}
  {@render children()}
{:else}
<div class="od-root">
  {#if !settings}
    <div class="od-blank" aria-busy="true"></div>
  {:else if !enabled}
    <div class="od-guard">
      <div class="guard-card">
        <h2>{$t("omnidisc.disabled_title")}</h2>
        <p>{$t("omnidisc.disabled_body")}</p>
        <button type="button" class="button primary" onclick={() => goto("/settings")}>{$t("omnidisc.open_settings")}</button>
      </div>
    </div>
  {:else if !showShell}
    {@render children()}
  {:else}
    <div class="od-shell">
      <InstanceRail />
      <ChannelList />
      <section class="od-main">
        <StorageBanner />
        {@render children()}
      </section>
      {#if showMembers}
        <MemberList />
      {/if}
    </div>
  {/if}
</div>
{/if}

<style>
  .od-root {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: transparent;
    color: var(--text);
  }

  .od-blank {
    flex: 1;
  }

  .od-guard {
    flex: 1;
    display: flex;
    align-items: flex-start;
    padding: var(--space-5);
  }

  .guard-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    max-width: 480px;
    padding: var(--space-5);
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
  }

  .guard-card h2 {
    margin: 0;
    font-size: var(--text-lg);
    color: var(--text);
  }

  .guard-card p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.5;
  }

  .guard-card .primary {
    background: var(--accent);
    color: var(--on-accent);
  }

  .od-shell {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: row;
  }

  .od-main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  @media (max-width: 900px) {
    .od-shell :global(.member-list) {
      display: none;
    }
  }

  @media (max-width: 700px) {
    .od-shell :global(.channel-list) {
      display: none;
    }
  }
</style>
