<script lang="ts" module>
  export type IconName =
    | "check"
    | "warning"
    | "lock"
    | "error"
    | "info"
    | "undo"
    | "copy"
    | "chevron"
    | "refresh"
    | "clock"
    | "skip";

  const PATHS: Record<IconName, string> = {
    check: "M10 1a9 9 0 1 1 0 18 9 9 0 0 1 0-18Zm4.1 5.3a1.1 1.1 0 0 0-1.56.05L8.9 10.3 7.4 8.8a1.1 1.1 0 1 0-1.56 1.56l2.3 2.3a1.1 1.1 0 0 0 1.6-.04l4.4-4.76a1.1 1.1 0 0 0-.04-1.56Z",
    warning: "M8.27 2.5a2 2 0 0 1 3.46 0l7 12.2A2 2 0 0 1 17 17.7H3a2 2 0 0 1-1.73-3l7-12.2ZM10 6.2a1.1 1.1 0 0 0-1.1 1.16l.25 4.3a.85.85 0 0 0 1.7 0l.25-4.3A1.1 1.1 0 0 0 10 6.2Zm0 7.3a1.15 1.15 0 1 0 0 2.3 1.15 1.15 0 0 0 0-2.3Z",
    lock: "M10 1.5a4.5 4.5 0 0 1 4.5 4.5v2h.5a2 2 0 0 1 2 2v6.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2h.5V6A4.5 4.5 0 0 1 10 1.5Zm0 2.2A2.3 2.3 0 0 0 7.7 6v2h4.6V6A2.3 2.3 0 0 0 10 3.7Zm0 7.3a1.4 1.4 0 0 0-.7 2.6v1.2a.7.7 0 1 0 1.4 0v-1.2A1.4 1.4 0 0 0 10 11Z",
    error: "M10 1a9 9 0 1 1 0 18 9 9 0 0 1 0-18ZM7.4 6.2a.85.85 0 0 0-1.2 1.2L8.8 10l-2.6 2.6a.85.85 0 1 0 1.2 1.2L10 11.2l2.6 2.6a.85.85 0 1 0 1.2-1.2L11.2 10l2.6-2.6a.85.85 0 1 0-1.2-1.2L10 8.8 7.4 6.2Z",
    info: "M10 1a9 9 0 1 1 0 18 9 9 0 0 1 0-18Zm0 7.3a1 1 0 0 0-1 1v4.9a1 1 0 1 0 2 0V9.3a1 1 0 0 0-1-1Zm0-3.6a1.2 1.2 0 1 0 0 2.4 1.2 1.2 0 0 0 0-2.4Z",
    undo: "M8.2 2.3a1 1 0 0 1 0 1.4L6.9 5H11a6.5 6.5 0 1 1 0 13H7a1.2 1.2 0 1 1 0-2.4h4a4.1 4.1 0 1 0 0-8.2H6.9l1.3 1.3a1 1 0 0 1 0 1.4 1.1 1.1 0 0 1-1.5 0L3.4 6.9a1.3 1.3 0 0 1 0-1.8l3.3-3.2a1.1 1.1 0 0 1 1.5.4Z",
    copy: "M7 1.5h8A2.5 2.5 0 0 1 17.5 4v8a2.5 2.5 0 0 1-2.5 2.5h-.5V7A3.5 3.5 0 0 0 11 3.5H4.6A2.5 2.5 0 0 1 7 1.5ZM4 5.5h7A2 2 0 0 1 13 7.5v9a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2Z",
    chevron: "M7.3 4.3a1.1 1.1 0 0 1 1.55 0l4.9 4.9a1.1 1.1 0 0 1 0 1.6l-4.9 4.9a1.1 1.1 0 1 1-1.55-1.56L11.4 10 7.3 5.86a1.1 1.1 0 0 1 0-1.56Z",
    refresh: "M10 2.2a7.8 7.8 0 0 1 6.4 3.35V3.8a1.1 1.1 0 1 1 2.2 0v4.6a1.1 1.1 0 0 1-1.1 1.1h-4.6a1.1 1.1 0 1 1 0-2.2h2A5.6 5.6 0 1 0 15.6 11a1.1 1.1 0 1 1 2.18.3A7.8 7.8 0 1 1 10 2.2Z",
    clock: "M10 1a9 9 0 1 1 0 18 9 9 0 0 1 0-18Zm0 3.6a1 1 0 0 0-1 1V10c0 .3.12.58.33.78l2.8 2.6a1 1 0 1 0 1.36-1.46L11 9.57V5.6a1 1 0 0 0-1-1Z",
    skip: "M10 1a9 9 0 1 1 0 18 9 9 0 0 1 0-18ZM6 9a1 1 0 1 0 0 2h8a1 1 0 1 0 0-2H6Z",
  };
</script>

<script lang="ts">
  let { name, size = 16, label }: { name: IconName; size?: number; label?: string } = $props();
</script>

<svg
  class="wmb-icon"
  width={size}
  height={size}
  viewBox="0 0 20 20"
  fill="currentColor"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
><path d={PATHS[name]} fill-rule="evenodd" /></svg>

<style>
  .wmb-icon {
    flex-shrink: 0;
    display: inline-block;
    vertical-align: middle;
  }
</style>
