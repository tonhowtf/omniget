<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { agentActions } from "$lib/native-agent/actions";

  type Props = {
    titleId: string;
    describedBy?: string;
    error?: string | null;
    busy?: boolean;
    denyLabel?: string;
    allowLabel: string;
    onDeny?: () => void;
    onAllow: () => void;
    width?: "md" | "lg";
    align?: "start" | "center";
    children: Snippet;
  };

  let {
    titleId,
    describedBy,
    error = null,
    busy = false,
    denyLabel,
    allowLabel,
    onDeny,
    onAllow,
    width = "md",
    align = "start",
    children,
  }: Props = $props();

  let dialog: HTMLDivElement | undefined = $state();

  function controls(): HTMLElement[] {
    if (!dialog) return [];
    return Array.from(
      dialog.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'),
    ).filter((el) => !el.hasAttribute("disabled"));
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Tab" || !dialog) return;
    const active = document.activeElement;
    const inside = !!active && dialog.contains(active);
    if (!inside && active?.closest('[role="alertdialog"]')) return;
    const items = controls();
    if (!items.length) {
      e.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    if (!inside || active === (e.shiftKey ? first : last)) {
      e.preventDefault();
      (e.shiftKey ? last : first).focus();
    }
  }

  onMount(() => {
    const node = dialog;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => {
      const active = document.activeElement;
      const free = !active || active === document.body || !!node?.contains(active);
      if (free && previous?.isConnected) previous.focus();
    };
  });

  $effect(() => {
    if (busy || !dialog) return;
    dialog.querySelector<HTMLElement>(".ad-row .btn")?.focus();
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="ad-backdrop" role="presentation"></div>
<div
  class="ad"
  class:lg={width === "lg"}
  class:center={align === "center"}
  role="alertdialog"
  aria-modal="true"
  aria-labelledby={titleId}
  aria-describedby={describedBy}
  bind:this={dialog}
>
  {@render children()}
  {#if error}<p class="ad-err" role="alert">{error}</p>{/if}
  <div class="ad-row">
    {#if onDeny}
      <button use:agentActions={[]} class="btn btn-secondary" disabled={busy} onclick={onDeny}>{denyLabel}</button>
    {/if}
    <button use:agentActions={[]} class="btn btn-primary" disabled={busy} onclick={onAllow}>
      {#if busy}<span class="spinner"></span>{/if}
      {allowLabel}
    </button>
  </div>
</div>

<style>
  .ad-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: rgb(0 0 0 / 0.32);
  }

  .ad {
    position: fixed;
    left: 50%;
    top: 50%;
    z-index: 1001;
    transform: translate(-50%, -50%);
    width: min(460px, calc(100vw - 32px));
    max-height: calc(100vh - 48px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    border-radius: var(--radius-xl, 16px);
    background: var(--surface-hi, var(--surface));
    color: var(--text);
    box-shadow: 0 24px 64px rgb(0 0 0 / 0.35), inset 0 0 0 var(--hairline, 1px) var(--content-border, transparent);
    text-align: start;
    animation: ad-in 160ms var(--ease-out, ease-out);
  }

  .ad.lg {
    width: min(500px, calc(100vw - 32px));
  }

  .ad.center {
    text-align: center;
  }

  @keyframes ad-in {
    from {
      opacity: 0;
      transform: translate(-50%, -48%) scale(0.98);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ad {
      animation: none;
    }
  }

  .ad-err {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--error);
  }

  .ad-row {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .ad.center .ad-row {
    justify-content: center;
  }
</style>
