<script lang="ts">
  /**
   * One tool as a small monogram tile coloured by how a component lands on
   * it: native (green), converted (blue), converted with losses (amber),
   * not supported (hollow, dimmed). The tooltip carries the reason/losses.
   */
  import { t } from "$lib/i18n";
  import { toolMono, type Compat } from "$lib/central/catalog";

  let {
    id,
    name,
    compat = null,
    size = 20,
    detected = false,
  }: { id: string; name: string; compat?: Compat | null; size?: number; detected?: boolean } = $props();

  let status = $derived(compat?.status ?? "unknown");
  let title = $derived.by(() => {
    const label = compat ? $t(`llm.central.agentkit.compat.${compat.status}`) : $t("llm.central.catalog.compat.unknown");
    const extra =
      compat?.status === "degraded"
        ? ` — ${compat.lost.join(", ")}`
        : compat?.status === "unsupported"
          ? ` — ${compat.reason}`
          : "";
    return `${name}: ${label}${extra}`;
  });
</script>

<span
  class="dot {status}"
  class:detected
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.42)}px"
  {title}
  role="img"
  aria-label={title}
  data-tool={id}>{toolMono(id, name)}</span>

<style>
  .dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border-radius: 28%;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: #fff;
    font-family: var(--font-body);
    box-shadow:
      inset 0 0 0 0.5px rgba(255, 255, 255, 0.25),
      inset 0 -1px 2px rgba(0, 0, 0, 0.15);
  }
  .native {
    background: linear-gradient(180deg, color-mix(in srgb, var(--success) 88%, white), var(--success));
  }
  .converted {
    background: linear-gradient(180deg, color-mix(in srgb, var(--blue) 85%, white), var(--blue));
  }
  .degraded {
    background: linear-gradient(180deg, color-mix(in srgb, var(--warning) 85%, white), var(--warning));
    color: var(--on-warning);
  }
  .unsupported,
  .unknown {
    background: transparent;
    color: var(--text-faint);
    box-shadow: inset 0 0 0 1px var(--border-hi);
  }
  .detected:not(.unsupported):not(.unknown) {
    outline: 1.5px solid color-mix(in srgb, var(--text) 40%, transparent);
    outline-offset: 1px;
  }
</style>
