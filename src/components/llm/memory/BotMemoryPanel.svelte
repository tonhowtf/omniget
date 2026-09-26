<script lang="ts">
  /**
   * "What it knows about you": the memories this bot reads in a direct
   * conversation — your profile (shared by your personal bots) and its own
   * private notes — grouped by kind, each with where it came from.
   */
  import { t } from "$lib/i18n";
  import MemoryScopeSection from "./MemoryScopeSection.svelte";
  import MemoryReach from "./MemoryReach.svelte";
  import {
    botScopes,
    getMemoryErrorKey,
    getScopeItems,
    isMemoryLoading,
    loadScopes,
  } from "$lib/stores/assist-memory-store.svelte";

  let { botId }: { botId: string } = $props();

  let keys = $derived(botScopes(botId));
  let userItems = $derived(getScopeItems("user"));
  let botItems = $derived(getScopeItems(`bot:${botId}`));
  let errorKey = $derived(getMemoryErrorKey());
  let empty = $derived(
    userItems.every((i) => i.status !== "active") && botItems.every((i) => i.status !== "active"),
  );

  $effect(() => {
    if (botId) void loadScopes(keys);
  });
</script>

<div class="bot-memory" data-bot={botId}>
  <header>
    <h2 class="block-title">{$t("assist.memory.panel_title")}</h2>
    <p class="dim">{$t("assist.memory.panel_lede")}</p>
  </header>

  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    <button type="button" class="button" disabled={isMemoryLoading()} onclick={() => loadScopes(keys)}>{$t("assist.memory.retry")}</button>
  {:else if isMemoryLoading() && empty}
    <p class="dim" role="status">{$t("assist.memory.loading")}</p>
  {:else if empty}
    <p class="notice" role="status">{$t("assist.memory.empty_bot")}</p>
  {/if}

  <MemoryScopeSection scopeKey="user" title={$t("assist.memory.scope_user")} lede={$t("assist.memory.scope_user_lede")} items={userItems} />
  <MemoryScopeSection scopeKey={`bot:${botId}`} title={$t("assist.memory.scope_bot_own")} lede={$t("assist.memory.scope_bot_lede")} items={botItems} />
  <MemoryReach compact />
</div>

<style>
  .bot-memory { display: grid; gap: 16px; }
  header { display: grid; gap: 4px; }
  .dim { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; max-width: 72ch; }
</style>
