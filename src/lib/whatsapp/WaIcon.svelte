<script lang="ts">
  let { name, size = 14 }: { name: string; size?: number } = $props();

  const PATHS: Record<string, string> = {
    refresh: "M20 11a8 8 0 0 0-14.9-3.9M4 5v4h4M4 13a8 8 0 0 0 14.9 3.9M20 19v-4h-4",
    x: "M6 6l12 12M18 6L6 18",
    clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2",
    check: "M5 12.5l4.5 4.5L19 7.5",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    download: "M12 4v11M7 10l5 5 5-5M5 20h14",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
    user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0",
    users: "M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2 20a7 7 0 0 1 14 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6 6 0 0 1 4 6",
    logout: "M15 4h4v16h-4M10 8l-4 4 4 4M6 12h10",
    photo: "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15 9h.01",
    video: "M4 6h11v12H4zM15 10l5-3v10l-5-3",
    file: "M6 3h8l4 4v14H6zM14 3v4h4",
    music: "M9 18V6l10-2v12M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM19 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
    mic: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3",
    sticker: "M20 12a8 8 0 1 1-8-8h8v8zM20 12h-5a3 3 0 0 0-3 3v5",
    pin: "M12 21s-7-6.2-7-11a7 7 0 0 1 14 0c0 4.8-7 11-7 11zM12 12.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
    contact: "M4 4h16v16H4zM12 12a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM7.5 17a4.5 4.5 0 0 1 9 0",
    poll: "M5 20V10M12 20V4M19 20v-7",
    send: "M4 12l16-8-6 16-3-7z M11 13l9-9",
    share: "M12 4v12M7 9l5-5 5 5M5 14v6h14v-6",
    warn: "M12 4l9 16H3zM12 10v4M12 17h.01",
    info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5M12 8h.01",
    history: "M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5M12 7v5l3 2",
    external: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
    phone: "M5 3h4l2 5-2.5 1.5a11 11 0 0 0 6 6L16 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 5a2 2 0 0 1 2-2z",
    qr: "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h2v2h-2zM18 18h2v2h-2zM18 14h2M14 18v2",
    wifi_off: "M2 2l20 20M8.5 16.5a5 5 0 0 1 7 0M5 13a10 10 0 0 1 5.2-2.8M19 13a10 10 0 0 0-2.3-1.7M2 9a15 15 0 0 1 4.3-2.7M22 9a15 15 0 0 0-11-3.9M12 20h.01",
  };

  let d = $derived(PATHS[name] ?? PATHS.info);
</script>

<svg
  class="wa-icon"
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
  .wa-icon {
    flex-shrink: 0;
    display: inline-block;
    vertical-align: -2px;
  }
</style>
