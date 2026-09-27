<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/washmybelly/api";
  import type { Health } from "$lib/washmybelly/disk";

  let { health }: { health: Health | null } = $props();
</script>

<section class="card">
  <h3>{$t("washmybelly.disk.health_title")}</h3>
  {#if !health}
    <p class="dim">{$t("washmybelly.disk.health_loading")}</p>
  {:else if !health.disks.length}
    <p class="dim">{$t("washmybelly.disk.health_none")}{#if health.reason} ({health.reason}){/if}</p>
  {:else}
    <ul>
      {#each health.disks as d, i (d.id ?? i)}
        <li>
          <span class="status {d.status}" aria-hidden="true"></span>
          <div class="body">
            <div class="title">{d.name ?? d.id ?? "?"} <span class="dim">{[d.media?.toUpperCase(), d.bus, d.size ? formatBytes(d.size) : null].filter(Boolean).join(" · ")}</span></div>
            <div class="sub">
              {$t(`washmybelly.disk.status.${d.status}`)}
              {#if d.temperature_c != null} · {d.temperature_c} °C{/if}
              {#if d.percent_used != null} · {$t("washmybelly.disk.wear", { count: d.percent_used })}{/if}
              {#if d.power_on_hours != null} · {$t("washmybelly.disk.hours", { count: d.power_on_hours.toLocaleString() })}{/if}
              {#if d.reason} · {d.reason}{/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; }
  h3 { margin: 0 0 6px; font-size: 13px; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: flex; gap: 10px; align-items: flex-start; padding: 8px 0; }
  li + li { border-top: var(--hairline, 1px) solid var(--separator); }
  .status { width: 10px; height: 10px; border-radius: 50%; margin-top: 4px; flex: none; background: var(--text-faint, #8e8e93); }
  .status.good { background: var(--success); }
  .status.warning { background: var(--warning); }
  .status.failing { background: var(--danger); }
  .body { min-width: 0; }
  .title { font-size: 13px; font-weight: 500; }
  .sub, .dim { font-size: 11.5px; color: var(--text-dim); font-weight: 400; }
  p { margin: 0; }
</style>
