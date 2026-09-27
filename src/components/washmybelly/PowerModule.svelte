<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { actionsOf, chargeChoices, managerKey, profileOptions, type Battery, type HoldState, type PowerStatus } from "$lib/washmybelly/power";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import BatteryHealthCard from "./BatteryHealthCard.svelte";
  import PowerSegmented from "./PowerSegmented.svelte";
  import PowerHoldCard from "./PowerHoldCard.svelte";
  import PowerConflictCard from "./PowerConflictCard.svelte";
  import PowerSleepList from "./PowerSleepList.svelte";

  let { caps }: { caps: Caps } = $props();

  let status = $state<PowerStatus | null>(null);
  let battery = $state<Battery | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let note = $state<string | null>(null);
  let busy = $state(false);
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      const [s, b] = await Promise.all([
        api.call<PowerStatus>("wmb_power_status"),
        caps.has_battery === false ? Promise.resolve(null) : api.call<Battery>("wmb_battery_health").catch(() => null),
      ]);
      status = s;
      battery = b;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  function review(id: string) {
    reviewIds = [id];
    reviewOpen = true;
  }

  async function hold(minutes: number) {
    busy = true;
    note = null;
    try {
      const h = await api.call<HoldState>("wmb_power_hold", { action: "hold", max_minutes: minutes });
      if (status) status = { ...status, hold: h };
    } catch (e) {
      note = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function release() {
    busy = true;
    try {
      const h = await api.call<HoldState>("wmb_power_hold", { action: "release" });
      if (status) status = { ...status, hold: h };
    } catch (e) {
      note = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function setProfile(p: string) {
    if (!status?.profile) return;
    busy = true;
    note = null;
    const prev = status.profile.current;
    status = { ...status, profile: { ...status.profile, current: p } };
    try {
      await api.ui("power.set_profile", { profile: p });
    } catch (e) {
      status = { ...status, profile: { ...status.profile!, current: prev } };
      note = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function batteryReport() {
    note = null;
    try {
      await api.ui<{ path: string }>("power.battery_report");
    } catch (e) {
      note = errorText(e);
    }
  }

  let os = $derived(status?.os ?? caps.os);
  let profiles = $derived(profileOptions(status));
  let conflictAction = $derived(actionsOf(status, "resolve_conflict")[0]);
  let lpm = $derived(actionsOf(status, "lowpowermode")[0]);
  let planActions = $derived(actionsOf(status, "set_scheme"));
  let ultimate = $derived(actionsOf(status, "ultimate")[0]);
  let applyNow = $derived([...actionsOf(status, "cpu_epp"), ...actionsOf(status, "cpu_governor")]);
  let limits = $derived(status?.charge_limits ?? []);
  let managerLabel = $derived($t(`washmybelly.power.manager.${managerKey(status?.manager)}`) as string);
  let hasBattery = $derived(!!battery?.present);

  function limitOptions(bat: string, end: number) {
    const opts = chargeChoices(status, bat).map((c) => ({ value: c.id, label: `${c.percent}%` }));
    opts.push({ value: `current:${end}`, label: `${end}%` });
    return opts.sort((a, b) => parseInt(a.label) - parseInt(b.label));
  }
</script>

<div class="power">
  <ModuleHeader tile={moduleById("power")} title={$t("washmybelly.modules.power") as string} subtitle={$t("washmybelly.power.subtitle") as string}>
    {#snippet meta()}
      {#if status}<span class="meta">{$t("washmybelly.power.managed_by", { name: managerLabel })}</span>{/if}
    {/snippet}
    {#snippet actions()}
      <button class="btn btn-sm" type="button" disabled={loading} onclick={load}>{$t("washmybelly.power.refresh")}</button>
    {/snippet}
  </ModuleHeader>

  {#if loading && !status}
    <EmptyState art="searching" title={$t("washmybelly.power.loading") as string} compact />
  {:else if error && !status}
    <EmptyState art="worried" title={$t("washmybelly.power.error_title") as string} body={error}>
      {#snippet action()}<button class="btn btn-sm" type="button" onclick={load}>{$t("washmybelly.power.refresh")}</button>{/snippet}
    </EmptyState>
  {:else if status}
    <div class="stack">
      {#if note}<p class="error" role="alert">{note}</p>{/if}

      {#if status.conflicts?.length}
        <PowerConflictCard conflicts={status.conflicts} action={conflictAction} onResolve={review} />
      {:else if os === "linux" && status.suggestion}
        <section class="card hint">
          <h3>{$t("washmybelly.power.none.title")}</h3>
          <p class="dim">{$t("washmybelly.power.none.body")}</p>
        </section>
      {/if}

      {#if hasBattery && battery}
        <BatteryHealthCard {battery}>
          {#snippet footer()}
            {#if limits.length}
              {#each limits as l (l.battery)}
                {@const opts = limitOptions(l.battery, l.end)}
                <div class="row">
                  <div class="row-text">
                    <span class="label">{$t("washmybelly.power.limit.title")}{#if limits.length > 1} · {l.battery}{/if}</span>
                    <span class="dim">{$t("washmybelly.power.limit.body")}</span>
                  </div>
                  {#if opts.length > 1}
                    <div class="seg-wrap">
                      <PowerSegmented options={opts} value={`current:${l.end}`} label={$t("washmybelly.power.limit.title") as string} onChange={(id) => review(id)} />
                    </div>
                  {:else}
                    <span class="value">{l.end}%</span>
                  {/if}
                </div>
              {/each}
            {/if}
            {#if os === "windows"}
              <div class="row">
                <span class="dim">{$t("washmybelly.power.battery.report_body")}</span>
                <button class="btn btn-sm" type="button" onclick={batteryReport}>{$t("washmybelly.power.battery.report")}</button>
              </div>
            {/if}
          {/snippet}
        </BatteryHealthCard>
      {/if}

      <section class="card">
        <h3>{$t("washmybelly.power.profile.title")}</h3>
        {#if os === "linux"}
          {#if profiles.length}
            <PowerSegmented
              options={profiles.map((p) => ({ value: p, label: $t(`washmybelly.power.profile.${p.replace("-", "_")}`) as string }))}
              value={status.profile?.current}
              label={$t("washmybelly.power.profile.title") as string}
              {busy}
              onChange={setProfile}
            />
            {#if status.profile?.holds?.length}
              <p class="dim">{$t("washmybelly.power.profile.held_by", { names: status.profile.holds.map((h) => h.app).join(", ") })}</p>
            {/if}
          {:else}
            <p class="dim">{$t("washmybelly.power.profile.no_switch")}</p>
          {/if}
          {#if status.cpu && status.cpu.cpus > 0}
            <dl class="knobs">
              {#if status.cpu.governor}<div><dt>{$t("washmybelly.power.cpu.governor")}</dt><dd>{status.cpu.governor}</dd></div>{/if}
              {#if status.cpu.epp}<div><dt>{$t("washmybelly.power.cpu.epp")}</dt><dd>{status.cpu.epp}</dd></div>{/if}
              {#if status.cpu.turbo != null}<div><dt>{$t("washmybelly.power.cpu.turbo")}</dt><dd>{status.cpu.turbo ? $t("washmybelly.power.on") : $t("washmybelly.power.off")}</dd></div>{/if}
              {#if status.cpu.platform_profile}<div><dt>{$t("washmybelly.power.cpu.platform")}</dt><dd>{status.cpu.platform_profile}</dd></div>{/if}
            </dl>
          {/if}
          {#if applyNow.length}
            <div class="apply">
              <span class="dim">{$t("washmybelly.power.cpu.apply_now")}</span>
              <div class="chips">
                {#each applyNow as a (a.id)}
                  <button class="btn btn-sm" type="button" onclick={() => review(a.id)}>{a.label}</button>
                {/each}
              </div>
            </div>
          {/if}
        {:else if os === "macos"}
          <div class="row">
            <div class="row-text">
              <span class="label">{$t("washmybelly.power.lpm.title")}</span>
              <span class="dim">{status.low_power_mode ? $t("washmybelly.power.lpm.on") : $t("washmybelly.power.lpm.off")}</span>
            </div>
            {#if lpm}
              <button class="btn btn-sm" type="button" onclick={() => review(lpm.id)}>
                {status.low_power_mode ? $t("washmybelly.power.lpm.turn_off") : $t("washmybelly.power.lpm.turn_on")}
              </button>
            {/if}
          </div>
        {:else if os === "windows"}
          {#if status.profile?.current}
            <p class="dim">{$t("washmybelly.power.overlay.current", { mode: $t(`washmybelly.power.overlay.${status.profile.current}`) })}</p>
          {/if}
          {#if status.plans?.length}
            <ul class="plans">
              {#each status.plans as p (p.guid)}
                {@const act = planActions.find((a) => a.args.guid === p.guid)}
                <li>
                  <span class="label">{p.name}</span>
                  {#if p.active}
                    <span class="tag">{$t("washmybelly.power.plans.active")}</span>
                  {:else if act}
                    <button class="btn btn-sm" type="button" onclick={() => review(act.id)}>{$t("washmybelly.power.plans.use")}</button>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if ultimate}
            <div class="row">
              <span class="dim">{$t("washmybelly.power.plans.ultimate_body")}</span>
              <button class="btn btn-sm" type="button" onclick={() => review(ultimate.id)}>{$t("washmybelly.power.plans.ultimate")}</button>
            </div>
          {/if}
        {/if}
      </section>

      <PowerHoldCard hold={status.hold} {os} {busy} onHold={hold} onRelease={release} />

      <PowerSleepList blockers={status.sleep_blockers ?? []} needsAdmin={!!status.sleep_blockers_need_admin} />
    </div>
  {/if}
</div>

<PlanReview kind="power" itemIds={reviewIds} bind:open={reviewOpen} onDone={load} onRestored={load} />

<style>
  .power { display: flex; flex-direction: column; min-height: 0; }
  .stack { display: flex; flex-direction: column; gap: 14px; }
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
  .hint { background: color-mix(in srgb, var(--accent) 7%, var(--surface)); }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  .meta { font-size: 12px; color: var(--text-dim); }
  .dim { margin: 0; font-size: 12px; color: var(--text-dim); }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .row + .row { margin-top: 8px; }
  .row-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .label { font-size: 13px; font-weight: 500; }
  .value { font-size: 13px; font-variant-numeric: tabular-nums; }
  .seg-wrap { min-width: 220px; }
  .knobs { display: flex; flex-wrap: wrap; gap: 6px 20px; margin: 0; }
  .knobs div { display: flex; flex-direction: column; gap: 1px; }
  dt { font-size: 11.5px; color: var(--text-dim); }
  dd { margin: 0; font-size: 13px; font-weight: 500; }
  .apply { display: flex; flex-direction: column; gap: 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .plans { list-style: none; margin: 0; padding: 0; }
  .plans li { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 7px 0; }
  .plans li + li { border-top: var(--hairline, 1px) solid var(--separator); }
  .tag { font-size: 11px; padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--success) 16%, transparent); color: var(--success); }
</style>
