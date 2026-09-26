<script lang="ts">
  /**
   * /llm/memory: every memory scope of the person — the profile shared by
   * their personal bots, each bot's private notes, each room's shared
   * memory — with history, export/import and what "forget" reaches.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgent, loadRoster } from "$lib/stores/llm-store.svelte";
  import MemoryScopeSection from "$components/llm/memory/MemoryScopeSection.svelte";
  import MemoryReach from "$components/llm/memory/MemoryReach.svelte";
  import {
    getMemoryErrorKey,
    getScopeItems,
    getScopes,
    isMemoryLoading,
    loadAllScopes,
  } from "$lib/stores/assist-memory-store.svelte";

  let showInactive = $state(false);
  let scopes = $derived(getScopes());
  let errorKey = $derived(getMemoryErrorKey());
  let others = $derived(scopes.filter((s) => s.scope_key !== "user"));

  function titleOf(key: string): string {
    if (key.startsWith("bot:")) {
      const id = key.slice(4);
      return $t("assist.memory.scope_bot", { name: getAgent(id)?.name ?? id }) as string;
    }
    if (key.startsWith("room:")) return $t("assist.memory.scope_room", { name: key.slice(5) }) as string;
    return $t("assist.memory.scope_user") as string;
  }

  function ledeOf(key: string): string {
    if (key.startsWith("bot:")) return $t("assist.memory.scope_bot_lede") as string;
    if (key.startsWith("room:")) return $t("assist.memory.scope_room_lede") as string;
    return $t("assist.memory.scope_user_lede") as string;
  }

  onMount(() => {
    void loadRoster();
    void loadAllScopes(showInactive);
  });
</script>

<svelte:head><title>{$t("assist.memory.page_title")}</title></svelte:head>

<div class="page page-wide memory-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("assist.memory.page_title")}</h1>
      <p class="page-lede">{$t("assist.memory.page_lede")}</p>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={showInactive} onchange={() => loadAllScopes(showInactive)} />
      <span>{$t("assist.memory.show_inactive")}</span>
    </label>
  </header>

  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    <button type="button" class="button" disabled={isMemoryLoading()} onclick={() => loadAllScopes(showInactive)}>{$t("assist.memory.retry")}</button>
  {/if}

  <MemoryScopeSection scopeKey="user" title={titleOf("user")} lede={ledeOf("user")} items={getScopeItems("user")} {showInactive} />

  {#each others as s (s.scope_key)}
    <MemoryScopeSection scopeKey={s.scope_key} title={titleOf(s.scope_key)} lede={ledeOf(s.scope_key)} items={getScopeItems(s.scope_key)} {showInactive} />
  {/each}

  {#if !isMemoryLoading() && others.length === 0 && !errorKey}
    <p class="notice" role="status">{$t("assist.memory.empty_page")} <a href="/llm/roster">{$t("assist.memory.empty_page_link")}</a></p>
  {/if}

  <MemoryReach />
</div>

<style>
  .memory-page { display: grid; gap: 16px; }
  .check { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; }
  .check input:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
