<script lang="ts">
  import { iconPath } from "$lib/canvas/roles/icons";

  let { icon, color, size = 16 }: { icon?: string; color?: string; size?: number } = $props();
  const d = $derived(iconPath(icon));
</script>

<span class="badge" style:--c={color ?? "var(--text-muted)"} style:--s="{size}px">
  {#if d}
    <svg
      width={size * 0.75}
      height={size * 0.75}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path {d} />
    </svg>
  {:else if icon}
    <span class="emoji">{icon}</span>
  {:else}
    <span class="dot"></span>
  {/if}
</span>

<style>
  .badge {
    display: inline-grid;
    place-items: center;
    width: var(--s);
    height: var(--s);
    border-radius: 6px;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 18%, transparent);
    flex: none;
  }
  .emoji {
    font-size: calc(var(--s) * 0.7);
    line-height: 1;
  }
  .dot {
    width: 40%;
    height: 40%;
    border-radius: 50%;
    background: var(--c);
  }
</style>
