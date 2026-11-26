<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings, saveSettings } from "#components/settings/settings-helpers.ts";
  import { HOLD_CHOICES, formatCountdown, holdProgress, remainingSecs, type HoldState } from "$lib/washmybelly/power";

  let {
    hold,
    os,
    busy = false,
    onHold,
    onRelease,
  }: { hold: HoldState | null | undefined; os: string; busy?: boolean; onHold: (minutes: number) => void; onRelease: () => void } = $props();

  let now = $state(Date.now());
  const timer = setInterval(() => (now = Date.now()), 1000);
  onDestroy(() => clearInterval(timer));

  let manual = $derived(!!hold?.active && !!hold?.until);
  let left = $derived(remainingSecs(hold?.until, now));
  let done = $derived(holdProgress(hold, now));
  let autos = $derived(hold?.auto_holders ?? []);
  let autoOn = $derived(getSettings()?.washmybelly?.auto_hold_power ?? false);
  let bodyKey = $derived(os === "linux" ? "washmybelly.power.hold.body_linux" : os === "windows" ? "washmybelly.power.hold.body_windows" : "washmybelly.power.hold.body_macos");
</script>

<section class="card">
  <div class="head">
    <div>
      <h3>{$t("washmybelly.power.hold.title")}</h3>
      <p class="dim">{$t(bodyKey)}</p>
    </div>
    {#if manual}
      <div class="pill" role="timer" aria-live="off" aria-label={$t("washmybelly.power.hold.left_aria", { time: formatCountdown(left) }) as string}>
        <span class="ring" style:--p={done} aria-hidden="true"></span>
        <span class="time">{formatCountdown(left)}</span>
        <button class="release" type="button" disabled={busy} onclick={onRelease}>{$t("washmybelly.power.hold.release")}</button>
      </div>
    {/if}
  </div>

  {#if !manual}
    <div class="choices">
      {#each HOLD_CHOICES as m (m)}
        <button class="btn btn-sm" type="button" disabled={busy} onclick={() => onHold(m)}>{$t("washmybelly.power.hold.for_minutes", { minutes: m })}</button>
      {/each}
    </div>
  {:else if hold?.reason}
    <p class="dim">{hold.reason}</p>
  {/if}

  <div class="auto-row">
    <div class="auto-text">
      <span class="auto-label" id="wmb-auto-hold-label">{$t("washmybelly.settings_auto_hold")}</span>
      <p class="dim auto" id="wmb-auto-hold-desc">{$t("washmybelly.settings_auto_hold_desc")}</p>
      {#if autos.length}
        <p class="dim auto">{$t("washmybelly.power.hold.auto_now", { count: autos.length })} · {autos.join(", ")}</p>
      {/if}
    </div>
    <button type="button" class="toggle" class:on={autoOn} role="switch" aria-checked={autoOn} aria-labelledby="wmb-auto-hold-label" aria-describedby="wmb-auto-hold-desc"
      onclick={() => saveSettings({ washmybelly: { auto_hold_power: !autoOn } })}><span class="toggle-knob"></span></button>
  </div>
  {#if hold?.error}<p class="error" role="alert">{hold.error}</p>{/if}
</section>

<style>
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  .dim { margin: 2px 0 0; font-size: 12px; color: var(--text-dim); }
  .auto { font-size: 11.5px; }
  .auto-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-top: 10px; border-top: var(--hairline, 1px) solid var(--separator); }
  .auto-text { min-width: 0; }
  .auto-label { font-size: 13px; color: var(--text); }
  .choices { display: flex; gap: 8px; flex-wrap: wrap; }
  .pill { display: inline-flex; align-items: center; gap: 8px; padding: 4px 4px 4px 8px; border-radius: 999px; background: color-mix(in srgb, var(--success) 14%, transparent); }
  .ring { width: 16px; height: 16px; border-radius: 50%; background: conic-gradient(var(--success) calc((1 - var(--p)) * 360deg), color-mix(in srgb, var(--success) 25%, transparent) 0); -webkit-mask: radial-gradient(circle, transparent 4px, #000 4.5px); mask: radial-gradient(circle, transparent 4px, #000 4.5px); }
  .time { font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; min-width: 3.2em; }
  .release { all: unset; cursor: pointer; font-size: 12px; font-weight: 500; padding: 3px 10px; border-radius: 999px; background: var(--surface); color: var(--text); box-shadow: 0 1px 2px rgb(0 0 0 / 0.12); }
  .release:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .release:disabled { opacity: 0.6; cursor: default; }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
</style>
