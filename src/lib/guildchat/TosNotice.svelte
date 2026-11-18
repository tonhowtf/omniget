<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";

  const KEY = "discord.tos_ack";

  function seen(): boolean {
    try {
      return localStorage.getItem(KEY) === "1";
    } catch {
      return false;
    }
  }

  let hidden = $state(seen());

  function dismiss() {
    hidden = true;
    try {
      localStorage.setItem(KEY, "1");
    } catch {
    }
  }
</script>

{#if !hidden}
  <div class="notice notice-info tos" role="note">
    <TgIcon name="shield" size={16} />
    <div class="notice-body">
      <div class="notice-title">{$t("guildchat.tos.title")}</div>
      <div class="notice-text">{$t("guildchat.tos.body")}</div>
      <ul>
        <li>{$t("guildchat.tos.tip_secondary")}</li>
        <li>{$t("guildchat.tos.tip_servers")}</li>
        <li>{$t("guildchat.tos.tip_dms")}</li>
      </ul>
    </div>
    <button class="btn btn-ghost btn-sm" onclick={dismiss}>{$t("guildchat.tos.ok")}</button>
  </div>
{/if}

<style>
  .tos {
    align-items: flex-start;
  }

  .tos ul {
    margin: var(--space-1) 0 0;
    padding-left: 1.2em;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.6;
  }
</style>
