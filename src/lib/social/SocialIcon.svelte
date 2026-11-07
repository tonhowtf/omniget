<script lang="ts">
  let { name, size = 16 }: { name: string; size?: number } = $props();

  const PATHS: Record<string, string> = {
    heart: "M12 20s-7-4.4-9.2-8.6C1.3 8.5 3 5 6.4 5c2 0 3.3 1.1 4 2.3.3.5 1 .5 1.2 0C12.3 6.1 13.6 5 15.6 5 19 5 20.7 8.5 19.2 11.4 17 15.6 12 20 12 20z",
    comment: "M4 5h16v11H9l-5 4V5z",
    repost: "M7 7h10l-2.5-2.5M17 17H7l2.5 2.5M17 7v4M7 17v-4",
    bookmark: "M7 4h10v16l-5-3.5L7 20V4z",
    views: "M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12zM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z",
    send: "M4 12l16-8-6 16-2.5-6.5L4 12z",
    home: "M4 11l8-7 8 7v9h-5v-6H9v6H4v-9z",
    search: "M11 5a6 6 0 1 0 0 12 6 6 0 0 0 0-12zM20 20l-4.5-4.5",
    at: "M16 12a4 4 0 1 1-1.2-2.8M16 9v4.5c0 1.4 2.5 1.8 3.2-.2.9-2.8-.4-6.8-4.6-8C9.8 4 5 7.2 5 12.2 5 17 9 20 13.5 19.5",
    user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4.5 20c1-3.6 4-5.5 7.5-5.5s6.5 1.9 7.5 5.5",
    play: "M8 5v14l11-7L8 5z",
    back: "M15 5l-7 7 7 7",
    external: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
    download: "M12 4v11M7 10l5 5 5-5M5 20h14",
    refresh: "M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6",
    plus: "M12 5v14M5 12h14",
    check: "M5 12.5l4.5 4.5L19 7",
    trash: "M5 7h14M10 7V5h4v2M7 7l1 13h8l1-13",
  };
  let d = $derived(PATHS[name] ?? PATHS.comment);
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path {d} /></svg>
