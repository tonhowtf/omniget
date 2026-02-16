<script lang="ts">
  let { platform, size = 18 }: { platform: string; size?: number } = $props();
</script>

{#if platform === "instagram"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="2" y="2" width="20" height="20" rx="5" ry="5" />
    <circle cx="12" cy="12" r="5" />
    <circle cx="17.5" cy="6.5" r="1.5" fill="currentColor" stroke="none" />
  </svg>
{:else if platform === "tiktok"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M9 12a4 4 0 1 0 4 4V4a5 5 0 0 0 5 5" />
  </svg>
{:else if platform === "twitter"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M4 4l16 16M20 4L4 20" />
  </svg>
{:else if platform === "reddit"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="12" cy="14" r="8" />
    <path d="M12 6V2" />
    <path d="M12 6l4-2" />
    <circle cx="9" cy="13" r="1" fill="currentColor" stroke="none" />
    <circle cx="15" cy="13" r="1" fill="currentColor" stroke="none" />
    <path d="M9 17c1.5 1 4.5 1 6 0" />
  </svg>
{:else if platform === "twitch"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M21 2H3v16h5v4l4-4h5l4-4V2z" />
    <path d="M11 11V7M15 11V7" />
  </svg>
{:else if platform === "pinterest"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="12" cy="10" r="8" />
    <path d="M10 14l-2 8" />
    <path d="M10 14c1 2 4 2 5 0" />
  </svg>
{:else if platform === "bluesky"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4C9 7 5 10 5 13a4 4 0 0 0 7 2.5" />
    <path d="M12 4c3 3 7 6 7 9a4 4 0 0 1-7 2.5" />
    <path d="M12 15.5V20" />
  </svg>
{:else if platform === "hotmart"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M7 4v16M17 4v16M7 12h10" />
  </svg>
{:else}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="12" cy="12" r="10" />
    <path d="M12 16v.01M12 8v4" />
  </svg>
{/if}

<style>
  svg {
    flex-shrink: 0;
    pointer-events: none;
  }
</style>
