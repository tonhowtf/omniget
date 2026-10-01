<script lang="ts">
  let { name, size = 14 }: { name: string; size?: number } = $props();

  const PATHS: Record<string, string> = {
    refresh: "M20 11a8 8 0 0 0-14.9-3.9M4 5v4h4M4 13a8 8 0 0 0 14.9 3.9M20 19v-4h-4",
    link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
    copy: "M9 9h10v10H9zM5 15V5h10",
    trash: "M4 7h16M10 11v6M14 11v6M6 7l1 12h10l1-12M9 7V4h6v3",
    check: "M5 12.5l4.5 4.5L19 7.5",
    x: "M6 6l12 12M18 6L6 18",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    download: "M12 4v11M7 10l5 5 5-5M5 20h14",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
    plug: "M9 3v5M15 3v5M7 8h10v4a5 5 0 0 1-10 0zM12 17v4",
    pin: "M12 3l7 7-4 1-4 4-1 5-3-3-4 4M8 13l3 3",
    play: "M8 5l11 7-11 7z",
    bolt: "M13 3L5 14h6l-1 7 8-11h-6z",
    shield: "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z",
    clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2",
    user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0",
    logout: "M15 4h4v16h-4M10 8l-4 4 4 4M6 12h10",
    save: "M5 4h11l3 3v13H5zM8 4v5h7V4M8 20v-6h8v6",
    photo: "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15 9h.01",
    video: "M4 6h11v12H4zM15 10l5-3v10l-5-3",
    file: "M6 3h8l4 4v14H6zM14 3v4h4",
    music: "M9 18V6l10-2v12M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM19 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
    chevron: "M9 6l6 6-6 6",
    share: "M12 4v12M7 9l5-5 5 5M5 14v6h14v-6",
    warn: "M12 4l9 16H3zM12 10v4M12 17h.01",
    info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5M12 8h.01",
    pause: "M8 5v14M16 5v14",
    next: "M6 5l9 7-9 7zM18 5v14",
    prev: "M18 5l-9 7 9 7zM6 5v14",
    volume: "M4 9h4l5-4v14l-5-4H4zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12",
    mute: "M4 9h4l5-4v14l-5-4H4zM17 9l5 6M22 9l-5 6",
    pip: "M3 5h18v14H3zM12 12h7v5h-7z",
    expand: "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5",
    external: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
    forward: "M14 5l7 7-7 7M21 12H9a6 6 0 0 0-6 6v1",
    upload: "M12 20V9M7 14l5-5 5 5M5 4h14",
    plus: "M12 5v14M5 12h14",
    edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
    clone: "M8 8h12v12H8zM4 16V4h12",
    eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
    inbox: "M3 13l3-8h12l3 8v6H3zM3 13h5l1 3h6l1-3h5",
    list: "M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01",
    tv: "M3 6h18v12H3zM8 21h8M10 10l5 2-5 2z",
    captions: "M3 5h18v14H3zM10 10.5a2 2 0 1 0 0 3M17 10.5a2 2 0 1 0 0 3",
    filter: "M4 5h16l-6 7.5V19l-4-2v-4.5z",
    convert: "M4 12a8 8 0 0 1 13.7-5.7L20 8.5M20 4v4.5h-4.5M20 12a8 8 0 0 1-13.7 5.7L4 15.5M4 20v-4.5h4.5",
    bot: "M12 3v3M6 8h12a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM9 13h.01M15 13h.01M2 13v2M22 13v2",
  };

  let d = $derived(PATHS[name] ?? PATHS.info);
</script>

<svg
  class="tg-icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path {d} /></svg>

<style>
  .tg-icon {
    flex-shrink: 0;
    display: inline-block;
    vertical-align: -2px;
  }
</style>
