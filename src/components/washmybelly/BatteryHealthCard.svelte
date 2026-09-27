<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import { capacityBars, healthTone, type Battery } from "$lib/washmybelly/power";

  /// Battery health: a battery that fills to the current charge (after
  /// ravikatiyar162's "Battery Card" on 21st.dev: the animated liquid became
  /// a CSS wave on a filled glyph), the health figures, and two bars that
  /// compare today's full charge with the factory capacity.
  let { battery, footer }: { battery: Battery; footer?: Snippet } = $props();

  let charge = $derived(Math.round(battery.charge_percent ?? 0));
  let bars = $derived(capacityBars(battery));
  let tone = $derived(healthTone(battery.health_percent));
  let charging = $derived(battery.state === "charging");
  let low = $derived(charge <= 20 && !charging);

  function remaining(min: number): string {
    const h = Math.floor(min / 60);
    const m = min % 60;
    return h > 0 ? ($t("washmybelly.power.battery.hours_minutes", { hours: h, minutes: m }) as string) : ($t("washmybelly.power.battery.minutes", { minutes: m }) as string);
  }
</script>

<section class="card">
  <div class="top">
    <div class="glyph" class:low class:charging role="img" aria-label={$t("washmybelly.power.battery.charge_aria", { percent: charge }) as string}>
      <div class="shell">
        <div class="liquid" style:width="calc((100% - 6px) * {charge / 100})"><span class="wave"></span></div>
        <span class="pct">{charge}%</span>
      </div>
      <span class="cap"></span>
    </div>
    <div class="headline">
      <h3>{$t("washmybelly.power.battery.title")}</h3>
      <p class="state">
        {$t(`washmybelly.power.battery.state.${battery.state}`)}
        {#if battery.time_remaining_min != null && battery.state === "discharging"} · {remaining(battery.time_remaining_min)}{/if}
        {#if battery.power_w != null} · {Math.abs(battery.power_w).toFixed(1)} W{/if}
      </p>
    </div>
  </div>

  <dl class="facts">
    {#if battery.health_percent != null}
      <div><dt>{$t("washmybelly.power.battery.health")}</dt><dd class={tone}>{Math.round(battery.health_percent)}%</dd></div>
    {/if}
    {#if battery.cycles != null}
      <div><dt>{$t("washmybelly.power.battery.cycles")}</dt><dd>{battery.cycles.toLocaleString()}</dd></div>
    {/if}
    {#if battery.temperature_c != null}
      <div><dt>{$t("washmybelly.power.battery.temperature")}</dt><dd>{battery.temperature_c.toFixed(1)} °C</dd></div>
    {/if}
  </dl>

  {#if bars}
    <div class="bars" aria-label={$t("washmybelly.power.battery.capacity_aria", { lost: bars.lost }) as string}>
      <div class="bar-row">
        <span class="bar-label">{$t("washmybelly.power.battery.design")}</span>
        <span class="track"><span class="fill design" style:width="100%"></span></span>
        <span class="bar-val">{bars.design.toLocaleString()} {bars.unit}</span>
      </div>
      <div class="bar-row">
        <span class="bar-label">{$t("washmybelly.power.battery.full_now")}</span>
        <span class="track"><span class="fill now {tone}" style:width="{(bars.full / bars.design) * 100}%"></span></span>
        <span class="bar-val">{bars.full.toLocaleString()} {bars.unit}</span>
      </div>
      {#if bars.lost > 0}<p class="dim">{$t("washmybelly.power.battery.lost", { lost: bars.lost })}</p>{/if}
    </div>
  {/if}

  {#if footer}<div class="footer">{@render footer()}</div>{/if}
</section>

<style>
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 14px 16px; display: flex; flex-direction: column; gap: 12px; }
  .top { display: flex; align-items: center; gap: 14px; }
  .glyph { display: flex; align-items: center; flex: none; }
  .shell { position: relative; width: 64px; height: 32px; border-radius: 8px; box-shadow: inset 0 0 0 2px var(--text-dim); overflow: hidden; display: grid; place-items: center; }
  .cap { width: 4px; height: 12px; margin-left: 2px; border-radius: 0 2px 2px 0; background: var(--text-dim); }
  /* Horizontal battery, so the liquid grows left to right. */
  .liquid { position: absolute; left: 3px; top: 3px; bottom: 3px; background: var(--success); border-radius: 5px; transition: width 0.6s ease; overflow: hidden; }
  .low .liquid { background: var(--danger); }
  .wave { position: absolute; inset: 0; background: linear-gradient(90deg, transparent, rgb(255 255 255 / 0.28), transparent); transform: translateX(-100%); animation: wave 2.4s ease-in-out infinite; }
  @keyframes wave { to { transform: translateX(100%); } }
  .pct { position: relative; z-index: 1; font-size: 12px; font-weight: 700; font-variant-numeric: tabular-nums; color: var(--text); text-shadow: 0 0 2px var(--surface); }
  .headline { min-width: 0; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  .state { margin: 2px 0 0; font-size: 12px; color: var(--text-dim); }
  .facts { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 0; }
  .facts div { display: flex; flex-direction: column; gap: 2px; }
  dt { font-size: 11.5px; color: var(--text-dim); }
  dd { margin: 0; font-size: 17px; font-weight: 600; font-variant-numeric: tabular-nums; }
  dd.good { color: var(--success); }
  dd.warn { color: var(--warning); }
  dd.bad { color: var(--danger); }
  .bars { display: flex; flex-direction: column; gap: 6px; }
  .bar-row { display: grid; grid-template-columns: minmax(90px, auto) 1fr auto; align-items: center; gap: 10px; font-size: 12px; }
  .bar-label { color: var(--text-dim); }
  .bar-val { font-variant-numeric: tabular-nums; color: var(--text-dim); }
  .track { height: 8px; border-radius: 999px; background: var(--surface-mut, var(--separator)); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 999px; }
  .fill.design { background: var(--text-faint, #8e8e93); opacity: 0.5; }
  .fill.now { background: var(--accent); }
  .fill.now.good { background: var(--success); }
  .fill.now.warn { background: var(--warning); }
  .fill.now.bad { background: var(--danger); }
  .dim { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .footer { border-top: var(--hairline, 1px) solid var(--separator); padding-top: 10px; }
  @media (prefers-reduced-motion: reduce) { .wave { animation: none; } .liquid { transition: none; } }
</style>
