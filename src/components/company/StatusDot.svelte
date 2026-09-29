<script lang="ts">
  /**
   * Coloured dot for a staff status, run state or issue status. Pulses while
   * `running`. `label` shows the translated status next to it.
   */
  import { t } from "$lib/i18n";
  import { statusTone } from "./company-ui";

  let {
    status,
    kind = "staff",
    label = false,
    size = 8,
  }: {
    status: string | null | undefined;
    /** Which i18n family names the status: staff_status, run_state, status (issue) or company_status. */
    kind?: "staff" | "run" | "issue" | "company";
    label?: boolean;
    size?: number;
  } = $props();

  const FAMILY = { staff: "staff_status", run: "run_state", issue: "status", company: "company_status" } as const;
  let tone = $derived(statusTone(status));
  let text = $derived(status ? $t(`company.common.${FAMILY[kind]}.${status}`) : "");
</script>

<span class="status-dot-wrap" title={label ? undefined : text}>
  <span
    class="dot {tone}"
    class:pulse={status === "running"}
    style:width="{size}px"
    style:height="{size}px"
    role={label ? undefined : "img"}
    aria-label={label ? undefined : text}
  ></span>
  {#if label}<span class="text">{text}</span>{/if}
</span>

<style>
  .status-dot-wrap { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .dot { position: relative; flex-shrink: 0; border-radius: 50%; background: var(--text-dim); }
  .dot.green { background: var(--green); }
  .dot.blue { background: var(--blue); }
  .dot.orange { background: var(--orange); }
  .dot.red { background: var(--red); }
  .dot.yellow { background: var(--warning); }
  .dot.gray { background: var(--text-dim); }
  .dot.faint { background: var(--text-faint); }
  .dot.pulse::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: inherit;
    animation: status-dot-pulse 1.6s ease-out infinite;
  }
  @keyframes status-dot-pulse {
    from { transform: scale(1); opacity: 0.7; }
    to { transform: scale(2.6); opacity: 0; }
  }
  @media (prefers-reduced-motion: reduce) { .dot.pulse::after { animation: none; } }
  .text { font-size: var(--text-xs); color: var(--text-muted); white-space: nowrap; }
</style>
