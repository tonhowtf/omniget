<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/research/api";
  import type { DevtoolsError } from "$lib/devtools/types";
  import ErrorNotice from "#components/devtools/ErrorNotice.svelte";

  let { configured = $bindable(null), usageTick = 0 }: { configured?: boolean | null; usageTick?: number } = $props();

  let key = $state("");
  let editing = $state(false);
  let busy = $state(false);
  let error = $state<DevtoolsError | null>(null);
  let remaining = $state<number | null>(null);
  let spent = $state(0);

  async function load() {
    try {
      configured = (await api.keyStatus()).configured;
    } catch (e) {
      error = api.parseError(e);
      configured = false;
    }
  }

  async function usage(refresh: boolean) {
    if (!configured) return;
    if (refresh) busy = true;
    try {
      const out = await api.tool<{ credits_remaining: number | null; credit_balance?: number; session_credits_used: number }>("research_usage", { refresh, limit: 1 });
      remaining = typeof out.credit_balance === "number" ? out.credit_balance : out.credits_remaining;
      spent = out.session_credits_used ?? 0;
      error = null;
    } catch (e) {
      if (refresh) error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  async function save() {
    if (!key.trim() || busy) return;
    busy = true;
    try {
      configured = (await api.keySet(key)).configured;
      key = "";
      editing = false;
      error = null;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      configured = (await api.keyClear()).configured;
      remaining = null;
      error = null;
    } catch (e) {
      error = api.parseError(e);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    void load();
  });

  $effect(() => {
    void usageTick;
    if (configured) void usage(false);
  });
</script>

<section class="dv-panel" aria-label={$t("research.key_title") as string}>
  <div class="dv-bar">
    <div class="dv-bar-group">
      <h2 class="dv-title">{$t("research.key_title")}</h2>
      {#if configured === null}
        <span class="spinner" aria-hidden="true"></span>
      {:else if configured}
        <span class="tag tag-success">{$t("research.key_saved")}</span>
      {:else}
        <span class="tag tag-warning">{$t("research.key_missing")}</span>
      {/if}
    </div>
    {#if configured}
      <div class="dv-bar-group">
        <span class="dv-meta">
          {remaining === null ? $t("research.usage_unknown") : $t("research.usage_remaining", { credits: remaining.toLocaleString() })}
          · {$t("research.usage_session", { credits: spent.toLocaleString() })}
        </span>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => void usage(true)}>{$t("research.usage_check")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (editing = !editing)}>{$t("research.key_replace")}</button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy} onclick={() => void remove()}>{$t("research.key_remove")}</button>
      </div>
    {/if}
  </div>
  {#if configured === false || editing}
    <form
      class="dv-row"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label class="dv-field wide">
        {$t("research.key_label")}
        <input class="input dv-mono" type="password" bind:value={key} placeholder={$t("research.key_placeholder") as string} autocomplete="off" spellcheck="false" />
      </label>
      <button type="submit" class="btn btn-primary" disabled={!key.trim() || busy}>
        {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
        {$t("research.key_save")}
      </button>
    </form>
    <p class="dv-meta">{$t("research.key_note")}</p>
  {/if}
  {#if error}<ErrorNotice {error} />{/if}
</section>
