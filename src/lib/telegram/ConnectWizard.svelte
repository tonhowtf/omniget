<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import RouteRow from "./RouteRow.svelte";
  import DoctorReport from "./DoctorReport.svelte";
  import PasteLinks from "./PasteLinks.svelte";
  import { net, errText, type DoctorReport as Report, type RouteView } from "./api";
  import { netState, scanNet, hasWorkingRoute, isScanning } from "./net.svelte";

  let { onDone, onOpenConnection }: { onDone: () => void; onOpenConnection: () => void } = $props();

  type Stage = "scanning" | "found" | "doctor";
  let stage = $state<Stage>("scanning");
  let report = $state<Report | null>(null);
  let doctorBusy = $state(false);
  let doctorError = $state<string | null>(null);
  let autoTimer: ReturnType<typeof setTimeout> | null = null;

  let routes = $derived<RouteView[]>(sortRoutes(netState.status?.routes ?? []));
  let chosen = $derived(netState.status?.active && netState.status.active.health.ok ? netState.status.active : null);
  let scanning = $derived(isScanning());

  function sortRoutes(list: RouteView[]): RouteView[] {
    const rank = (v: RouteView) => (v.active ? 0 : v.health.ok === true ? 1 : v.health.ok == null ? 2 : 3);
    return [...list].sort((a, b) => rank(a) - rank(b) || (a.health.rtt_ms ?? 1e9) - (b.health.rtt_ms ?? 1e9));
  }

  async function start() {
    stage = "scanning";
    report = null;
    clearAuto();
    const s = await scanNet();
    if (hasWorkingRoute(s)) {
      stage = "found";
      autoTimer = setTimeout(onDone, 1800);
    } else {
      stage = "doctor";
      await runDoctor();
    }
  }

  async function runDoctor() {
    doctorBusy = true;
    doctorError = null;
    try {
      report = await net.doctor();
      netState.status = report.status;
      if (report.verdict === "connected") {
        stage = "found";
      }
    } catch (e) {
      doctorError = errText(e);
    } finally {
      doctorBusy = false;
    }
  }

  function clearAuto() {
    if (autoTimer) clearTimeout(autoTimer);
    autoTimer = null;
  }

  onMount(() => {
    void start();
  });

  onDestroy(clearAuto);
</script>

<section class="wizard">
  <img class="hero" src="/telegram/hero.webp" alt="" width="996" height="689" decoding="async" draggable="false" />
  <header class="w-head">
    <div class="w-icon" data-stage={stage}>
      {#if stage === "scanning"}<span class="spinner"></span>{:else if stage === "found"}<TgIcon name="check" size={20} />{:else}<TgIcon name="plug" size={20} />{/if}
    </div>
    <div>
      <h2>{$t(`telegram.wizard.${stage}_title`)}</h2>
      <p>{$t(`telegram.wizard.${stage}_text`)}</p>
    </div>
  </header>

  {#if stage === "found"}
    <div class="chosen">
      {#if chosen}
        <span class="c-label">{$t("telegram.wizard.chosen")}</span>
        <strong>{chosen.route.name}</strong>
        {#if chosen.health.rtt_ms != null}<span class="c-ms">{$t("telegram.status.ms", { ms: chosen.health.rtt_ms })}</span>{/if}
      {:else}
        <strong>{$t("telegram.status.connected_plain")}</strong>
      {/if}
      <button class="btn btn-primary" onclick={() => { clearAuto(); onDone(); }}>{$t("telegram.wizard.continue")}</button>
    </div>
  {/if}

  {#if routes.length}
    <div>
      <span class="group-label">{$t("telegram.wizard.found_routes", { count: routes.length })}</span>
      <div class="group routes">
        {#each routes as view (view.route.id)}
          <RouteRow {view} pending={scanning} actions={false} />
        {/each}
      </div>
    </div>
  {:else if scanning}
    <div class="group routes">
      {#each [0, 1, 2] as i (i)}
        <div class="skel-row"><span class="skeleton circle"></span><span class="skeleton line"></span></div>
      {/each}
    </div>
  {/if}

  {#if stage === "doctor"}
    <div>
      <span class="group-label">{$t("telegram.doctor.title")}</span>
      {#if doctorBusy}
        <p class="muted"><span class="spinner"></span> {$t("telegram.doctor.running")}</p>
      {:else if report}
        <DoctorReport {report} />
      {:else if doctorError}
        <p class="field-error">{doctorError}</p>
      {/if}
    </div>

    <div class="fixes">
      <div class="fix surface-card">
        <h3>{$t("telegram.wizard.fix_app_title")}</h3>
        <ol>
          <li>{$t("telegram.wizard.fix_app_1")}</li>
          <li>{$t("telegram.wizard.fix_app_2")}</li>
          <li>{$t("telegram.wizard.fix_app_3")}</li>
        </ol>
        <button class="btn btn-secondary btn-sm" onclick={() => void start()} disabled={scanning}>
          <TgIcon name="refresh" /> {$t("telegram.wizard.try_again")}
        </button>
      </div>
      <div class="fix surface-card">
        <h3>{$t("telegram.wizard.fix_paste_title")}</h3>
        <p class="muted">{$t("telegram.wizard.fix_paste_text")}</p>
        <PasteLinks
          rows={3}
          onAdded={(r) => {
            if (r.added.length && hasWorkingRoute(netState.status)) stage = "found";
          }}
        />
      </div>
    </div>

    <div class="w-foot">
      <button class="btn btn-ghost btn-sm" onclick={onOpenConnection}>{$t("telegram.wizard.choose_manually")}</button>
      <button class="btn btn-ghost btn-sm" onclick={() => { clearAuto(); onDone(); }}>{$t("telegram.wizard.skip")}</button>
    </div>
  {/if}
</section>

<style>
  .wizard {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 720px;
    width: 100%;
    margin-inline: auto;
  }

  .hero {
    display: block;
    align-self: center;
    width: clamp(220px, 42vw, 440px);
    height: auto;
    margin-bottom: calc(var(--space-2) * -1);
    user-select: none;
    pointer-events: none;
  }

  .w-head {
    display: flex;
    gap: var(--space-4);
    align-items: center;
  }

  .w-icon {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-full);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .w-icon[data-stage="found"] {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }

  .w-icon[data-stage="doctor"] {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
  }

  h2 {
    margin: 0 0 2px;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .w-head p {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .chosen {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--success) 10%, transparent);
  }

  .chosen strong {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }

  .c-label {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .c-ms {
    font-variant-numeric: tabular-nums;
    color: var(--success);
    font-size: var(--text-sm);
  }

  .routes {
    max-height: 340px;
    overflow-y: auto;
  }

  .skel-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
  }

  .skel-row .circle {
    width: 20px;
    height: 20px;
  }

  .skel-row .line {
    flex: 1;
    height: 12px;
    border-radius: var(--radius-xs);
  }

  .fixes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: var(--space-3);
  }

  .fix {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    align-items: flex-start;
  }

  .fix h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .fix ol {
    margin: 0;
    padding-left: 1.2em;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.6;
  }

  .fix :global(.paste) {
    width: 100%;
  }

  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  .w-foot {
    display: flex;
    justify-content: space-between;
  }
</style>
