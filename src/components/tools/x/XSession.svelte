<script lang="ts">
  /** Linha de sessão do X: quem está logado, entrar e recarregar os query IDs. */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { loginToX, session as loadSession, xErr, type XSession } from "$lib/tools/x";

  let { required = false, onchange }: { required?: boolean; onchange?: (s: XSession) => void } = $props();
  let s = $state<XSession | null>(null);
  let busy = $state<string | null>(null);

  export async function refresh() {
    try {
      s = await loadSession();
      onchange?.(s);
    } catch (e) {
      s = { logged_in: false, user_id: null, user: null, query_ids_cached: 0, query_ids_age_secs: null };
      showToast("error", xErr(e));
    }
  }

  async function login() {
    busy = "login";
    try {
      const ok = await loginToX();
      if (ok) showToast("success", $t("tools.x.login_ok") as string);
      else showToast("error", $t("tools.x.login_fail") as string);
      await refresh();
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  async function refreshIds() {
    busy = "ids";
    try {
      const n = await invoke<number>("tool_x_query_ids_refresh");
      showToast("success", `${n} ${$t("tools.x.query_ids")}`);
      await refresh();
    } catch (e) {
      showToast("error", xErr(e));
    } finally {
      busy = null;
    }
  }

  onMount(refresh);
</script>

<div class="group-row">
  <div class="group-row-content">
    <div class="group-row-title">
      {#if !s}…{:else if s.logged_in}
        {#if s.user}<img class="avatar" src={s.user.avatar} alt="" width="22" height="22" /> {s.user.name} <span class="dim">@{s.user.handle}</span>{:else}{$t("tools.x.logged_in")}{/if}
      {:else}{$t("tools.x.logged_out")}{/if}
    </div>
    <div class="group-row-sub">
      {#if s?.logged_in}{$t("tools.x.session_hint_in")}{:else if required}{$t("tools.x.session_hint_required")}{:else}{$t("tools.x.session_hint_out")}{/if}
      {#if s && s.query_ids_cached > 0} · {s.query_ids_cached} {$t("tools.x.query_ids")}{/if}
    </div>
  </div>
  <div class="group-row-trailing btn-row">
    <button class="btn btn-ghost btn-sm" type="button" disabled={busy !== null} title={$t("tools.x.refresh_ids_hint")} onclick={refreshIds}>{busy === "ids" ? $t("tools.common.working") : $t("tools.x.refresh_ids")}</button>
    <button class="btn btn-sm" class:btn-primary={!s?.logged_in} class:btn-secondary={s?.logged_in} type="button" disabled={busy !== null} onclick={login}>{busy === "login" ? $t("tools.common.working") : s?.logged_in ? $t("tools.x.relogin") : $t("tools.x.login")}</button>
  </div>
</div>

<style>
  .avatar { width: 22px; height: 22px; border-radius: 50%; vertical-align: -5px; margin-right: 4px; }
  .dim { color: var(--text-muted); font-weight: 400; }
</style>
