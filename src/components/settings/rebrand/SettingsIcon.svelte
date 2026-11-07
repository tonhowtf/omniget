<script lang="ts" module>
  export type SettingsIconName =
    | "general" | "appearance" | "downloads" | "ai" | "integrations" | "advanced"
    | "search" | "chevron" | "external" | "alert" | "check"
    | "power" | "window-close" | "globe" | "person" | "more"
    | "gauge" | "plug" | "blocks" | "canvas" | "key"
    | "palette" | "text" | "captions" | "folder" | "keyboard" | "cookie" | "network" | "magnet" | "speed" | "wrench" | "pet" | "chat" | "terminal" | "film";
</script>

<script lang="ts">
  let { name, size = 18 }: { name: SettingsIconName; size?: 18 | 20 } = $props();

  const PATHS: Record<SettingsIconName, string> = {
    general: "M4 6h9m4 0h3M15 4v4M4 12h3m4 0h9M9 10v4M4 18h11m4 0h1M17 16v4",
    appearance: "M12 3a9 9 0 1 0 0 18c1.2 0 1.6-.9 1.2-1.7-.5-1 .1-2.3 1.3-2.3H16a5 5 0 0 0 5-5c0-5-4-9-9-9zM7.5 11.5h.01M10.5 7.5h.01M15.5 7.5h.01",
    downloads: "M12 4v10m0 0 4-4m-4 4-4-4M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3",
    ai: "M12 4l1.8 5.2L19 11l-5.2 1.8L12 18l-1.8-5.2L5 11l5.2-1.8z",
    integrations: "M9 3v5M15 3v5M7 8h10v3a5 5 0 0 1-10 0zM12 16v5",
    advanced: "M14.7 6.3a4 4 0 0 0-5.4 5.1L4 16.7V20h3.3l5.3-5.3a4 4 0 0 0 5.1-5.4l-2.5 2.5-2.4-.6-.6-2.4z",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
    chevron: "M9 6l6 6-6 6",
    external: "M14 4h6v6M20 4l-9 9M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4",
    alert: "M12 4l9 16H3zM12 10v4M12 17h.01",
    check: "M5 12.5l4.5 4.5L19 7",
    power: "M12 3v9M7 6.3a8 8 0 1 0 10 0",
    "window-close": "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM3 9h18M10 12.5l4 4M14 12.5l-4 4",
    globe: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.500 5.600 12 3z",
    person: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4.500 20.500c.8-3.600 3.800-5.500 7.500-5.500s6.700 1.900 7.500 5.500",
    more: "M5 12h.01M12 12h.01M19 12h.01",
    gauge: "M4 17a9 9 0 1 1 16 0M12 15l4-5M12 15h.01",
    plug: "M9 3v5M15 3v5M7 8h10v3a5 5 0 0 1-10 0zM12 16v5",
    blocks: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 16.500h7M16.500 13v7",
    canvas: "M4 5h16v14H4zM8 9h4v3H8zM14 13h3v3h-3zM12 10.500l2 3",
    key: "M15 4a5 5 0 0 0-4.700 6.700L4 17v3h3l1-1v-2h2l1-1v-2l.300-.300A5 5 0 1 0 15 4zM16.500 7.500h.01",
    palette: "M12 3a9 9 0 0 0 0 18c1.100 0 1.600-.900 1.200-1.800-.500-1 .200-2.200 1.300-2.200H17a4 4 0 0 0 4-4c0-5.500-4-10-9-10zM7.500 11h.01M10 7h.01M15 7h.01",
    text: "M4 7V5h16v2M12 5v14M9 19h6",
    captions: "M3 6h18v12H3zM7 11h4M13 11h4M7 14.500h7M16 14.500h1",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    keyboard: "M3 7h18v10H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10",
    cookie: "M12 3a9 9 0 1 0 9 9 3 3 0 0 1-3-3 3 3 0 0 1-3-3 3 3 0 0 1-3-3zM9 10h.01M14 15h.01M9 15.500h.01",
    network: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3.600 9h16.800M3.600 15h16.800M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18",
    magnet: "M6 4v8a6 6 0 0 0 12 0V4h-4v8a2 2 0 0 1-4 0V4zM6 8h4M14 8h4",
    speed: "M13 3 5 14h6l-1 7 8-11h-6z",
    wrench: "M14.700 6.300a4 4 0 0 0-5.400 5.100L4 16.700V20h3.300l5.300-5.300a4 4 0 0 0 5.100-5.400l-2.500 2.500-2.400-.600-.600-2.400z",
    pet: "M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM16 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM5 14a2 2 0 1 0 0-4M19 14a2 2 0 1 0 0-4M12 12c-3 0-5 3.500-5 5.500 0 1.500 1 2.500 2.500 2.500 1 0 1.700-.500 2.500-.500s1.500.500 2.500.500c1.500 0 2.500-1 2.500-2.500 0-2-2-5.500-5-5.500z",
    chat: "M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-8l-5 4v-4H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z",
    terminal: "M4 5h16v14H4zM8 10l3 2-3 2M13 15h3",
    film: "M4 5h16v14H4zM8 5v14M16 5v14M4 9h4M4 15h4M16 9h4M16 15h4",
  };
</script>

<svg
  class="settings-icon"
  class:directional={name === "chevron"}
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
><path d={PATHS[name]} /></svg>

<style>
  .settings-icon { flex: none; display: block; }
  .directional:dir(rtl) { transform: scaleX(-1); }
</style>
