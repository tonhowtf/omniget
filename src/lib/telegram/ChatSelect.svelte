<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import type { TelegramChat } from "./api";
  import { chatsState, loadChats } from "./session.svelte";

  let {
    value = $bindable(null),
    exclude = null,
    filter = null,
    label,
  }: {
    value: TelegramChat | null;
    exclude?: TelegramChat | null;
    filter?: ((c: TelegramChat) => boolean) | null;
    label: string;
  } = $props();

  const keyOf = (c: TelegramChat) => `${c.chat_type}:${c.id}`;
  let q = $state("");

  let options = $derived.by(() => {
    const needle = q.trim().toLowerCase();
    return chatsState.chats.filter(
      (c) =>
        (!exclude || keyOf(c) !== keyOf(exclude)) &&
        (!filter || filter(c)) &&
        (!needle || c.title.toLowerCase().includes(needle) || (value && keyOf(c) === keyOf(value))),
    );
  });

  onMount(() => {
    if (!chatsState.loaded) void loadChats(true);
  });

  function pick(key: string) {
    value = chatsState.chats.find((c) => keyOf(c) === key) ?? null;
  }
</script>

<div class="cs">
  {#if chatsState.chats.length > 12}
    <input class="input input-search" type="search" bind:value={q} placeholder={$t("telegram.browser.search_chats")} aria-label={$t("telegram.browser.search_chats")} />
  {/if}
  <select class="input" value={value ? keyOf(value) : ""} onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)} aria-label={label}>
    <option value="" disabled>{chatsState.loading ? $t("telegram.common.loading") : $t("telegram.export.pick_chat")}</option>
    {#each options as c (keyOf(c))}
      <option value={keyOf(c)}>{c.title} · {$t(`telegram.browser.chat_type.${c.chat_type}`)}</option>
    {/each}
  </select>
  {#if chatsState.next}
    <button type="button" class="btn btn-ghost btn-sm" onclick={() => void loadChats()} disabled={chatsState.loading}>{$t("telegram.browser.more_chats")}</button>
  {/if}
</div>

<style>
  .cs {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: stretch;
  }

  .cs .btn {
    align-self: flex-start;
  }
</style>
