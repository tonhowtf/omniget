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
{:else if platform === "vimeo"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M3 8.5C3 8.5 5 7 7 7c3 0 3.5 3 3 6s-3 7-5 7c-1.5 0-2.5-1-2.5-1" />
    <path d="M21 4c0 2.5-3 10.5-5.5 10.5S13 11 13 11" />
  </svg>
{:else if platform === "telegram"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M15 10l-4 4l6 6l4 -16l-18 7l4 2l2 6l3 -4" />
  </svg>
{:else if platform === "youtube"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="2" y="4" width="20" height="16" rx="4" />
    <path d="M10 9l5 3l-5 3z" />
  </svg>
{:else if platform === "soundcloud"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M2 17l.5-5 .5 5M5 17l.5-8 .5 8M8 17l.5-10 .5 10M11 17l.5-7 .5 7" />
    <path d="M14 17V7a3 3 0 0 1 3-3c2 0 4 1 5 3v10H14z" />
  </svg>
{:else if platform === "facebook"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M18 2h-3a5 5 0 0 0-5 5v3H7v4h3v8h4v-8h3l1-4h-4V7a1 1 0 0 1 1-1h3z" />
  </svg>
{:else if platform === "dailymotion"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4v7" />
    <circle cx="14" cy="14" r="5" />
    <path d="M4 4v2" />
  </svg>
{:else if platform === "bilibili"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="3" y="6" width="18" height="14" rx="3" />
    <path d="M8 6l-2-3M16 6l2-3" />
    <circle cx="9" cy="13" r="1" fill="currentColor" stroke="none" />
    <circle cx="15" cy="13" r="1" fill="currentColor" stroke="none" />
  </svg>
{:else if platform === "snapchat"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 3c-3 0-5 2.5-5 6v1c-1 0-2 .5-2 1s1 1 2 1c-1 2-3 3-3 3h16s-2-1-3-3c1 0 2-.5 2-1s-1-1-2-1v-1c0-3.5-2-6-5-6z" />
  </svg>
{:else if platform === "tumblr"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M14 21h-4a3 3 0 0 1-3-3v-5H5V9a4 4 0 0 0 4-4V3h4v5h3v5h-3v4a1 1 0 0 0 1 1h3v3z" />
  </svg>
{:else if platform === "vk"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M2 8h3l3 8 3-8h3" />
    <path d="M18 8v4c0 3-1 4-2 5l3 3" />
  </svg>
{:else if platform === "generic"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M9 15l6 -6" />
    <path d="M11 6l.463 -.536a5 5 0 0 1 7.071 7.072L18 13" />
    <path d="M13 18l-.397 .534a5.068 5.068 0 0 1 -7.127 0a4.972 4.972 0 0 1 0 -7.071L6 11" />
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
