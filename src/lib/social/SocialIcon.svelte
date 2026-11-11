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
    spark: "M12 3l2.2 6.8L21 12l-6.8 2.2L12 21l-2.2-6.8L3 12l6.8-2.2L12 3z",
    plus: "M12 5v14M5 12h14",
    check: "M5 12.5l4.5 4.5L19 7",
    trash: "M5 7h14M10 7V5h4v2M7 7l1 13h8l1-13",
    users: "M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2.5 20c.8-3.4 3.3-5 6.5-5s5.700 1.600 6.500 5M16 4.300a3.500 3.500 0 0 1 0 6.400M18 15.300c1.800.700 3 2.300 3.500 4.700",
    chart: "M4 20V4M4 20h16M8 16v-5M12 16V8M16 16v-3",
    radar: "M12 12l6-6M12 3a9 9 0 1 0 9 9M12 7.500a4.500 4.500 0 1 0 4.500 4.500",
    clock: "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 8v4.500l3 2",
    filter: "M4 5h16l-6 7v6l-4 2v-8L4 5z",
    star: "M12 4l2.500 5.200 5.700.700-4.200 3.900 1.100 5.600L12 16.600 6.900 19.400 8 13.800 3.800 9.900l5.700-.700L12 4z",
    eyeoff: "M4 4l16 16M9.500 5.900A9.500 9.500 0 0 1 12 5.500c6 0 9.500 6.500 9.500 6.500a17 17 0 0 1-3 3.700M6.300 7.800A16 16 0 0 0 2.500 12S6 18.500 12 18.500c1.300 0 2.500-.300 3.500-.700",
    calendar: "M5 6h14v14H5V6zM5 10h14M9 4v4M15 4v4",
    up: "M12 4l7 8h-4v8H9v-8H5l7-8z",
  };
  let d = $derived(PATHS[name] ?? PATHS.comment);
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path {d} /></svg>
