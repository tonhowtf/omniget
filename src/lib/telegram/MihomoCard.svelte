<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import { net, errText, type MihomoStatus } from "./api";
  import { refreshNet, netState } from "./net.svelte";

  let st = $state<MihomoStatus | null>(null);
  let busy = $state(false);

  let kinds = $derived(
    Object.entries(st?.kinds ?? {})
      .map(([k, n]) => `${k} ×${n}`)
      .join(", "),
  );

  async function load() {
    try {
      st = await net.mihomoStatus();
    } catch {
      st = null;
    }
  }

  onMount(load);

  // New links or a subscription can bring nodes only mihomo speaks.
  $effect(() => {
    void netState.status;
    void load();
  });

  async function toggle(on: boolean) {
    busy = true;
    try {
      st = await net.mihomoSet(on);
      if (on) showToast("success", $t("telegram.mihomo.enabled_toast", { count: st.nodes.length }) as string);
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
      await load();
    } finally {
      busy = false;
    }
  }

  async function restart() {
    busy = true;
    try {
      st = await net.mihomoRestart();
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(key: string) {
    try {
      await net.mihomoRemoveNode(key);
      await load();
      await refreshNet();
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

{#if st && st.supported_platform && (st.nodes.length > 0 || st.enabled)}
  <section>
    <div class="sec-head">
      <h3>{$t("telegram.mihomo.title")}</h3>
      {#if st.enabled}
        <button class="btn btn-ghost btn-sm" onclick={restart} disabled={busy}>
          {#if busy}<span class="spinner"></span>{:else}<TgIcon name="refresh" />{/if}
          {$t("telegram.mihomo.restart")}
        </button>
      {/if}
    </div>
    <p class="sec-sub">{$t("telegram.mihomo.hint", { kinds })}</p>

    {#if st.enabled == null && st.nodes.length > 0}
      <div class="offer">
        <p>{$t("telegram.mihomo.offer", { count: st.nodes.length })}</p>
        <div class="offer-actions">
          <button class="btn btn-primary btn-sm" onclick={() => toggle(true)} disabled={busy}>
            {#if busy}<span class="spinner"></span>{/if}
            {$t("telegram.mihomo.turn_on")}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={() => toggle(false)} disabled={busy}>{$t("telegram.mihomo.not_now")}</button>
        </div>
      </div>
    {:else}
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <span class="group-row-title">{$t("telegram.mihomo.switch")}</span>
            <span class="group-row-sub">
              {#if st.installing}{$t("telegram.mihomo.installing")}
              {:else if st.running}{$t("telegram.mihomo.running", { version: st.version ?? st.pinned_version })}
              {:else if st.enabled}{$t("telegram.mihomo.stopped")}
              {:else}{$t("telegram.mihomo.off")}{/if}
            </span>
          </div>
          <button class="toggle" class:on={!!st.enabled} role="switch" aria-checked={!!st.enabled} aria-label={$t("telegram.mihomo.switch") as string} disabled={busy} onclick={() => toggle(!st?.enabled)}><span class="toggle-knob"></span></button>
        </div>
        {#each st.nodes as n (n.key)}
          <div class="group-row">
            <div class="group-row-content">
              <span class="group-row-title">{n.name}</span>
              <span class="group-row-sub">{n.kind} · {n.server}:{n.port}</span>
            </div>
            <div class="group-row-trailing">
              <button class="btn btn-ghost btn-sm btn-icon" title={$t("telegram.net.remove")} aria-label={$t("telegram.net.remove")} onclick={() => remove(n.key)}><TgIcon name="trash" /></button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {#if st.error}<p class="err">{st.error}</p>{/if}
  </section>
{/if}

<style>
  .offer {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: 12px;
    background: var(--fill-1, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .offer p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }
  .offer-actions {
    display: flex;
    gap: var(--space-2);
  }
  .err {
    margin: var(--space-2) 0 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
</style>
