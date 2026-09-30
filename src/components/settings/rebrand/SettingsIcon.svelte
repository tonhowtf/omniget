<script lang="ts" module>
  export type SettingsIconName =
    | "general" | "appearance" | "downloads" | "ai" | "integrations" | "advanced"
    | "search" | "chevron" | "external" | "alert" | "check";
</script>

<script lang="ts">
  let { name, size = 18 }: { name: SettingsIconName; size?: 18 | 20 } = $props();

  // Original outline paths on a 24 grid, one stroke language for the whole settings shell.
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
  /* Only the navigation chevron follows reading direction. */
  .directional:dir(rtl) { transform: scaleX(-1); }
</style>
