<script lang="ts">
  import type { Snippet } from "svelte";

  /// Pibble illustration from static/washmybelly/{art}.webp. Until the art
  /// ships (or if it fails to load) a soap-bubble tile stands in, tinted by
  /// mood, so the state never looks broken.
  type Art = "sleeping" | "happy" | "worried" | "searching";
  let {
    art = "sleeping",
    title,
    body = "",
    action,
    size = 132,
    compact = false,
  }: { art?: Art; title: string; body?: string; action?: Snippet; size?: number; compact?: boolean } = $props();

  let failed = $state(false);
  $effect(() => {
    void art;
    failed = false;
  });
</script>

<div class="wmb-empty" class:compact>
  {#if !failed}
    <img
      class="wmb-empty-art"
      src="/washmybelly/{art}.webp"
      alt=""
      width={compact ? size * 0.7 : size}
      height={compact ? size * 0.7 : size}
      onerror={() => (failed = true)}
    />
  {:else}
    <span class="wmb-empty-fallback wmb-mood-{art}" style:--s="{compact ? 64 : 88}px" aria-hidden="true">
      <svg viewBox="0 0 64 64" fill="currentColor">
        <circle cx="26" cy="34" r="16" opacity="0.95" />
        <circle cx="44" cy="22" r="9" opacity="0.7" />
        <circle cx="46" cy="44" r="6" opacity="0.55" />
        <circle cx="20" cy="28" r="4.5" fill="#fff" opacity="0.55" />
        {#if art === "sleeping"}
          <path d="M18 36h6l-6 6h6M28 32h4l-4 4h4" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" opacity="0.9" />
        {:else if art === "happy"}
          <path d="M19 38c3 4 11 4 14 0" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" opacity="0.9" />
        {:else if art === "worried"}
          <path d="M19 41c3-3 11-3 14 0" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" opacity="0.9" />
        {:else}
          <circle cx="26" cy="34" r="6" fill="none" stroke="#fff" stroke-width="3" opacity="0.9" />
          <path d="M30.5 38.5l4 4" stroke="#fff" stroke-width="3.2" stroke-linecap="round" opacity="0.9" />
        {/if}
      </svg>
    </span>
  {/if}
  <h3>{title}</h3>
  {#if body}<p>{body}</p>{/if}
  {#if action}<div class="wmb-empty-action">{@render action()}</div>{/if}
</div>

<style>
  .wmb-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-5);
    text-align: center;
  }
  .wmb-empty.compact {
    padding: var(--space-5) var(--space-4);
  }
  .wmb-empty-art {
    object-fit: contain;
    margin-bottom: var(--space-2);
    filter: drop-shadow(0 8px 14px rgba(var(--shadow-ink), var(--elev-alpha-1)));
  }
  .wmb-empty-fallback {
    --tone: var(--purple);
    display: grid;
    place-items: center;
    width: var(--s);
    height: var(--s);
    margin-bottom: var(--space-2);
    border-radius: 28%;
    color: var(--tone);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
  }
  .wmb-empty-fallback svg {
    width: 72%;
    height: 72%;
  }
  .wmb-mood-happy { --tone: var(--success); }
  .wmb-mood-worried { --tone: var(--warning); }
  .wmb-mood-searching { --tone: var(--blue); }
  h3 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: var(--track-snug);
    color: var(--text);
  }
  p {
    margin: 0;
    max-width: 400px;
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--text-dim);
  }
  .wmb-empty-action {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
</style>
