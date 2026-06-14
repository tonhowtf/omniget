<script lang="ts">
  let { icon, iconSvg, size = 18 }: { icon: string; iconSvg?: string; size?: number } = $props();
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if iconSvg}
    {#each iconSvg.split(" M").map((d, i) => (i === 0 ? d : "M" + d)) as pathD}
      <path d={pathD} />
    {/each}
  {:else if icon === "home"}
    <path d="M3 12L12 3l9 9" />
    <path d="M5 10v9a1 1 0 001 1h3v-5h6v5h3a1 1 0 001-1v-9" />
  {:else if icon === "downloads"}
    <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
    <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
  {:else if icon === "marketplace"}
    <path d="M3 21h18" />
    <path d="M3 7v1a3 3 0 0 0 6 0V7" />
    <path d="M9 7v1a3 3 0 0 0 6 0V7" />
    <path d="M15 7v1a3 3 0 0 0 6 0V7" />
    <path d="M3 7h18l-1.5-4H4.5z" />
    <path d="M5 21V10" />
    <path d="M19 21V10" />
  {:else if icon === "settings"}
    <path d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
    <circle cx="12" cy="12" r="3" />
  {:else if icon === "about"}
    <circle cx="12" cy="12" r="10" />
    <path d="M12 16v-4m0-4h.01" />
  {:else}
    <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
  {/if}
</svg>
