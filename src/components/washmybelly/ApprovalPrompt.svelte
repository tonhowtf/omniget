<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { formatBytes, type PlanPreview, type ReceiptSummary } from "$lib/washmybelly/api";

  type Pending = {
    approval: { id: string; target: string; client: string; created: number };
    detail: (PlanPreview & Partial<ReceiptSummary>) | null;
  };

  let queue = $state<Pending[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let current = $derived(queue[0] ?? null);
  let isRestore = $derived(current?.approval.target.startsWith("restore:") ?? false);

  async function refresh() {
    try {
      queue = await invoke<Pending[]>("washmybelly_approvals");
    } catch {
      queue = [];
    }
  }

  async function answer(allow: boolean) {
    if (!current) return;
    busy = true;
    error = null;
    try {
      await invoke("washmybelly_approval_answer", { id: current.approval.id, allow });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      await refresh();
    }
  }

  onMount(() => {
    void refresh();
    const un = listen("washmybelly-approval", () => void refresh());
    return () => void un.then((f) => f());
  });
</script>

{#if current}
  <div class="wmb-ap-backdrop" role="presentation"></div>
  <div class="wmb-ap" role="alertdialog" aria-modal="true" aria-labelledby="wmb-ap-title">
    <img src="/washmybelly/worried.webp" alt="" width="72" height="72" />
    <h3 id="wmb-ap-title">
      {isRestore ? $t("washmybelly.approval.title_restore", { client: current.approval.client }) : $t("washmybelly.approval.title", { client: current.approval.client })}
    </h3>
    {#if current.detail && !isRestore}
      <p class="wmb-ap-sum">
        {$t("washmybelly.plan.items", { count: current.detail.item_count })}{#if current.detail.total_bytes > 0} · {$t("washmybelly.plan.total", { size: formatBytes(current.detail.total_bytes) })}{/if}
        {#if current.detail.needs_admin}<br />{$t("washmybelly.plan.needs_admin")}{/if}
      </p>
      <ul class="wmb-ap-items">
        {#each current.detail.items.slice(0, 8) as it (it.id)}
          <li>
            <span class="lbl">{it.label}{#if it.action?.path}<small>{it.action.path}</small>{/if}</span>
            {#if it.bytes > 0}<span class="sz">{formatBytes(it.bytes)}</span>{/if}
          </li>
        {/each}
        {#if current.detail.item_count > 8}
          <li class="more">{$t("washmybelly.approval.more", { count: current.detail.item_count - 8 })}</li>
        {/if}
      </ul>
    {/if}
    <p class="wmb-ap-hint">{$t("washmybelly.approval.hint")}</p>
    {#if error}<p class="wmb-ap-err">{error}</p>{/if}
    <div class="wmb-ap-row">
      <button class="btn btn-secondary" disabled={busy} onclick={() => answer(false)}>{$t("washmybelly.approval.deny")}</button>
      <button class="btn btn-primary" disabled={busy} onclick={() => answer(true)}>{$t("washmybelly.approval.allow")}</button>
    </div>
  </div>
{/if}

<style>
  .wmb-ap-backdrop { position: fixed; inset: 0; background: rgb(0 0 0 / 0.32); z-index: 1000; }
  .wmb-ap {
    position: fixed; left: 50%; top: 50%; transform: translate(-50%, -50%); z-index: 1001;
    width: min(460px, 92vw); padding: 22px 22px 18px; border-radius: 16px; text-align: center;
    background: var(--bg-elevated, var(--primary)); box-shadow: 0 24px 64px rgb(0 0 0 / 0.35);
  }
  h3 { margin: 6px 0 6px; font-size: 16px; }
  .wmb-ap-sum { margin: 0 0 10px; font-size: 13px; color: var(--text-secondary, var(--gray)); }
  .wmb-ap-items { list-style: none; margin: 0 0 10px; padding: 0; text-align: left; max-height: 200px; overflow: auto; border-radius: 10px; background: color-mix(in srgb, currentColor 5%, transparent); }
  .wmb-ap-items li { display: flex; justify-content: space-between; gap: 8px; padding: 6px 10px; font-size: 12px; }
  .wmb-ap-items li + li { border-top: 1px solid var(--separator, var(--button-elevated)); }
  .wmb-ap-items .sz { color: var(--text-secondary, var(--gray)); font-variant-numeric: tabular-nums; }
  .wmb-ap-items .lbl { display: flex; flex-direction: column; min-width: 0; }
  .wmb-ap-items small { color: var(--text-secondary, var(--gray)); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .wmb-ap-items .more { color: var(--text-secondary, var(--gray)); }
  .wmb-ap-hint { margin: 0; font-size: 12px; color: var(--text-secondary, var(--gray)); }
  .wmb-ap-err { color: #ff453a; font-size: 12px; }
  .wmb-ap-row { display: flex; justify-content: center; gap: 10px; margin-top: 14px; }
</style>
