<script lang="ts">
  /**
   * Linha de conta compartilhada por todas as tools do Instagram: qual
   * sessão (cookies capturados pela extensão) está em uso, quem é o usuário
   * logado e como capturar quando não há nenhuma.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { openUrl } from "$lib/tools/rt";
  import { igState, loadAccounts, setAccount, type UserInfo } from "$lib/tools/ig.svelte";

  let { onready }: { onready?: (me: UserInfo | null) => void } = $props();
  let loading = $state(true);
  let error = $state("");

  async function refresh() {
    loading = true;
    error = "";
    try {
      const list = await loadAccounts();
      if (igState.slug && list.some((a) => a.slug === igState.slug && a.has_session)) {
        igState.me = await invoke<UserInfo>("tool_ig_whoami", { slug: igState.slug });
      } else {
        igState.me = null;
      }
    } catch (e) {
      igState.me = null;
      error = typeof e === "string" ? e : String((e as { message?: string })?.message ?? e);
    } finally {
      loading = false;
      onready?.(igState.me);
    }
  }

  onMount(refresh);

  async function change(slug: string) {
    setAccount(slug);
    await refresh();
  }

  let usable = $derived(igState.accounts.filter((a) => a.has_session));
</script>

<section>
  <span class="group-label">{$t("tools.ig.account.label")}</span>
  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        {#if loading}
          <div class="group-row-title">{$t("tools.ig.account.checking")}</div>
        {:else if igState.me}
          <div class="who">
            {#if igState.me.profile_pic_url}<img class="avatar" src={igState.me.profile_pic_url} alt="" width="36" height="36" />{/if}
            <div>
              <div class="group-row-title">@{igState.me.username} {#if igState.me.is_verified}✓{/if}</div>
              <div class="group-row-sub">{igState.me.full_name} · {igState.me.follower_count.toLocaleString()} {$t("tools.ig.common.followers")} · {igState.me.following_count.toLocaleString()} {$t("tools.ig.common.following")}</div>
            </div>
          </div>
        {:else}
          <div class="group-row-title">{$t("tools.ig.account.none_title")}</div>
          <div class="group-row-sub">{error || $t("tools.ig.account.none_desc")}</div>
        {/if}
      </div>
      <div class="group-row-trailing btn-row">
        {#if usable.length > 1}
          <select class="select" value={igState.slug} onchange={(e) => change((e.currentTarget as HTMLSelectElement).value)}>
            {#each usable as a (a.slug)}
              <option value={a.slug}>{a.alias}</option>
            {/each}
          </select>
        {/if}
        {#if !igState.me && !loading}
          <button class="btn btn-secondary btn-sm" type="button" onclick={() => openUrl("https://www.instagram.com/")}>{$t("tools.ig.account.open_site")}</button>
          <a class="btn btn-secondary btn-sm" href="/settings?tab=cookies">{$t("tools.ig.account.cookies")}</a>
        {/if}
        <button class="btn btn-ghost btn-sm" type="button" disabled={loading} onclick={refresh} title={$t("tools.common.refresh")}>↻</button>
      </div>
    </div>
  </div>
</section>

<style>
  .who { display: flex; align-items: center; gap: var(--space-3); }
  .avatar { border-radius: 50%; object-fit: cover; flex: none; }
</style>
