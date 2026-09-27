<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import { errorText } from "$lib/washmybelly/format";
  import Icon, { type IconName } from "./Icon.svelte";
  import Sheet from "./Sheet.svelte";

  /// Generic confirmation for a dangerous screen action (end a process,
  /// restore a receipt). `onConfirm` may be async; the dialog stays open and
  /// shows the error if it throws, and closes when it resolves.
  let {
    open = $bindable(false),
    title,
    body = "",
    confirmLabel,
    cancelLabel,
    destructive = false,
    tone,
    onConfirm,
    children,
  }: {
    open?: boolean;
    title: string;
    body?: string;
    confirmLabel: string;
    cancelLabel?: string;
    destructive?: boolean;
    tone?: "warning" | "danger" | "info";
    onConfirm: () => unknown | Promise<unknown>;
    children?: Snippet;
  } = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);
  let icon = $derived<IconName>(tone === "info" ? "info" : "warning");
  let toneClass = $derived(tone ?? (destructive ? "danger" : "warning"));

  $effect(() => {
    if (open) error = null;
  });

  async function confirm() {
    busy = true;
    error = null;
    try {
      await onConfirm();
      open = false;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Sheet bind:open label={title} locked={busy} width={420}>
  <div class="wmb-cd">
    <span class="wmb-cd-icon tone-{toneClass}" aria-hidden="true"><Icon name={icon} size={26} /></span>
    <h3>{title}</h3>
    {#if body}<p>{body}</p>{/if}
    {#if children}{@render children()}{/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <button class="btn btn-secondary" disabled={busy} onclick={() => (open = false)}>{cancelLabel ?? $t("washmybelly.plan.cancel")}</button>
    <button class="btn {destructive ? 'wmb-btn-danger' : 'btn-primary'}" disabled={busy} data-autofocus onclick={confirm}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
      {confirmLabel}
    </button>
  {/snippet}
</Sheet>

<style>
  .wmb-cd {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    text-align: center;
  }
  .wmb-cd-icon {
    --tone: var(--warning);
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    margin-bottom: var(--space-1);
    border-radius: 14px;
    color: var(--tone);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
  }
  .tone-danger { --tone: var(--danger); }
  .tone-info { --tone: var(--info); }
  h3 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: var(--track-snug);
  }
  p {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
</style>
