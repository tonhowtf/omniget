<script lang="ts">
  /**
   * Inspector of the 3D city (M3 §6): the selected building or lot, its
   * intent and construction, the proposals that touched it (author, role,
   * reason, violations — never a chain of thought), approve/refuse for works
   * waiting for a person, the publish policy and the council round.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as city from "$lib/city3d/api";
  import type { CityLog, CityStatus, CouncilReport, LogEntry, RegionData } from "$lib/city3d/api";
  import type { Building, BuildingSpec, Lot, PatchOp } from "$lib/city3d/types";

  interface Selection {
    kind: "building" | "lot" | "prop" | "module" | "ground";
    region: [number, number];
    id: string | null;
    owner?: string | null;
    building?: Building | null;
    lot?: Lot | null;
  }

  let { selection = null }: { selection?: Selection | null } = $props();

  let status = $state<CityStatus | null>(null);
  let logData = $state<CityLog>({ proposals: [], works: [] });
  let regionData = $state<RegionData | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let council = $state<CouncilReport | null>(null);
  let councilBusy = $state(false);
  let open = $state<string | null>(null);

  const region = $derived<[number, number]>(selection?.region ?? [0, 0]);
  const buildingId = $derived(
    selection?.kind === "building" ? selection.id : (selection?.building?.id ?? selection?.lot?.building ?? null),
  );
  const building = $derived<Building | null>(
    selection?.building ?? regionData?.blueprint?.buildings.find((b) => b.id === buildingId) ?? null,
  );
  const lot = $derived<Lot | null>(
    selection?.lot ??
      regionData?.blueprint?.lots.find((l) => l.id === (selection?.kind === "lot" ? selection.id : building?.lot)) ??
      null,
  );
  const spec = $derived<BuildingSpec | null>(
    regionData?.program.buildings.find((b) => b.id === buildingId) ?? null,
  );
  const work = $derived(logData.works.find((w) => w.id === buildingId) ?? null);
  const pending = $derived(logData.proposals.filter((p) => p.state === "pending_review"));
  const touching = $derived(
    logData.proposals.filter((p) => touches(p, [buildingId, lot?.id ?? null])).slice(0, 30),
  );
  const policy = $derived(status?.city?.publish_policy ?? "auto");
  const schedule = $derived(status?.city?.settings?.council_every_min ?? null);
  const local = $derived(status?.mode !== "server");

  function touches(p: LogEntry, ids: (string | null)[]): boolean {
    const want = ids.filter((x): x is string => !!x);
    if (!want.length) return false;
    return p.ops.some((op: PatchOp) => {
      const o = op as Record<string, unknown>;
      return want.includes(o.id as string) || want.includes(o.lot as string);
    });
  }

  function when(ms: number): string {
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return String(ms);
    }
  }

  function who(p: LogEntry): string {
    const role = p.author.role ? $t(`city3d.found.role_${p.author.role}`) : $t(`city3d.found.author_${p.author.kind}`);
    return `${role} · ${p.author.name}`;
  }

  async function refresh(): Promise<void> {
    try {
      const [s, l] = await Promise.all([city.status(), city.log(200)]);
      status = s;
      logData = l;
      regionData = s.city ? await city.region(region) : null;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function decide(p: LogEntry, approve: boolean): Promise<void> {
    busy = true;
    try {
      const r = await city.review(p.id, approve);
      if ("state" in r && r.state === "stale") error = $t("city3d.found.stale") as string;
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function runCouncil(): Promise<void> {
    councilBusy = true;
    council = null;
    error = null;
    try {
      council = await city.councilTick({ region });
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      councilBusy = false;
    }
  }

  async function setPolicy(p: "auto" | "review"): Promise<void> {
    try {
      await city.setPolicy(p);
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  async function setSchedule(value: string): Promise<void> {
    try {
      await city.councilSchedule(value ? Number(value) : null);
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    void region;
    void refresh();
  });

  onMount(() => {
    const stopTicker = city.startWorksTicker(5000);
    let stops: (() => void)[] = [];
    void Promise.all([
      city.onChanged(() => void refresh()),
      city.onCouncil(() => void refresh()),
    ])
      .then((u) => (stops = u))
      .catch(() => {});
    return () => {
      stopTicker();
      stops.forEach((s) => s());
    };
  });
</script>

<div class="inspector">
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if !selection || (!building && !lot)}
    <p class="note">{$t("city3d.found.select_hint")}</p>
  {/if}

  {#if building}
    <section class="card">
      <p class="title">{spec?.intent.label ?? building.id}</p>
      <dl>
        <dt>{$t("city3d.found.use")}</dt><dd>{building.use}</dd>
        <dt>{$t("city3d.found.floors")}</dt><dd>{building.floors}</dd>
        <dt>{$t("city3d.found.roof")}</dt><dd>{building.roof.kind}</dd>
        <dt>{$t("city3d.found.style")}</dt><dd>{building.style} · {building.palette}</dd>
        {#if building.owner_ref}<dt>{$t("city3d.found.owner")}</dt><dd>{$t("city3d.found.owned")}</dd>{/if}
      </dl>
      {#if spec}
        <p class="field-label">{$t("city3d.found.intent")}</p>
        <p class="intent">
          {[spec.intent.use, spec.intent.shape, spec.intent.roof, spec.intent.tower ? "tower" : null, spec.intent.balcony ? "balcony" : null, spec.intent.awning ? "awning" : null]
            .filter(Boolean)
            .join(" · ")}
        </p>
      {/if}
      {#if (work?.progress ?? building.progress) < 100}
        {@const progress = work?.progress ?? building.progress}
        <p class="field-label">{$t("city3d.found.work", { count: progress })}</p>
        <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={progress}>
          <span style:width="{progress}%"></span>
        </div>
      {/if}
    </section>
  {/if}

  {#if lot}
    <section class="card">
      <p class="title">{$t("city3d.found.lot", { id: lot.id })}</p>
      <dl>
        <dt>{$t("city3d.found.zone")}</dt><dd>{lot.zone}</dd>
        <dt>{$t("city3d.found.size")}</dt><dd>{lot.rect[2]}×{lot.rect[3]} ({lot.rect[2] * 2}×{lot.rect[3] * 2} m)</dd>
        <dt>{$t("city3d.found.front")}</dt><dd>{lot.front.toUpperCase()}</dd>
        <dt>{$t("city3d.found.owner")}</dt><dd>{lot.owner_ref ? $t("city3d.found.owned") : $t("city3d.found.public")}</dd>
      </dl>
    </section>
  {/if}

  {#if touching.length}
    <section>
      <p class="field-label">{$t("city3d.found.history")}</p>
      <ul class="log">
        {#each touching as p (p.id)}{@render entry(p)}{/each}
      </ul>
    </section>
  {/if}

  {#if pending.length}
    <section>
      <p class="field-label">{$t("city3d.found.pending", { count: pending.length })}</p>
      <ul class="log">
        {#each pending as p (p.id)}{@render entry(p)}{/each}
      </ul>
    </section>
  {/if}

  {#if logData.works.length}
    <section>
      <p class="field-label">{$t("city3d.found.works", { count: logData.works.length })}</p>
      <ul class="works">
        {#each logData.works as w (w.id)}
          <li>
            <span>{w.label ?? w.id}</span>
            <div class="bar small"><span style:width="{w.progress}%"></span></div>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if status?.city}
    <section class="card">
      <p class="field-label">{$t("city3d.found.council")}</p>
      {#if local}
        <div class="row">
          <span>{$t("city3d.found.policy")}</span>
          <div class="segmented" role="group">
            <button type="button" class:active={policy === "auto"} aria-pressed={policy === "auto"} onclick={() => setPolicy("auto")}>{$t("city3d.found.policy_auto")}</button>
            <button type="button" class:active={policy === "review"} aria-pressed={policy === "review"} onclick={() => setPolicy("review")}>{$t("city3d.found.policy_review")}</button>
          </div>
        </div>
        <label class="row">
          <span>{$t("city3d.found.schedule")}</span>
          <select class="input" value={schedule ? String(schedule) : ""} onchange={(e) => setSchedule((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">{$t("city3d.found.schedule_off")}</option>
            <option value="10">{$t("city3d.found.schedule_every", { count: 10 })}</option>
            <option value="30">{$t("city3d.found.schedule_every", { count: 30 })}</option>
            <option value="60">{$t("city3d.found.schedule_every", { count: 60 })}</option>
          </select>
        </label>
      {/if}
      <button type="button" class="button" disabled={councilBusy} onclick={runCouncil}>
        {councilBusy ? $t("city3d.found.council_running") : $t("city3d.found.council_run")}
      </button>
      {#if council}
        <ul class="steps">
          {#each council.steps as s, i (i)}
            <li>
              <strong>{$t(`city3d.found.role_${s.role}`)}</strong>
              <span class="state {s.state}">{$t(`city3d.found.state_${s.state}`)}</span>
              {#if s.reason}<span class="msg">{s.reason}</span>{/if}
              {#if s.message}<span class="msg dim">{s.message}</span>{/if}
            </li>
          {/each}
        </ul>
        <p class="note">{council.model}</p>
      {/if}
    </section>
  {/if}
</div>

{#snippet entry(p: LogEntry)}
  <li class="entry">
    <div class="head">
      <span class="state {p.state}">{$t(`city3d.found.state_${p.state}`)}</span>
      <span class="who">{who(p)}</span>
      <span class="when">{when(p.at_ms)}</span>
    </div>
    {#if p.reason}<p class="msg">{p.reason}</p>{/if}
    {#if p.resolution}<p class="msg dim">{p.resolution}</p>{/if}
    {#if p.violations.length}
      <button type="button" class="link" aria-expanded={open === p.id} onclick={() => (open = open === p.id ? null : p.id)}>
        {$t("city3d.found.violations", {
          errors: p.violations.filter((v) => v.severity === "error").length,
          warnings: p.violations.filter((v) => v.severity !== "error").length,
        })}
      </button>
      {#if open === p.id}
        <ul class="violations">
          {#each p.violations as v, i (i)}
            <li class:warning={v.severity !== "error"}><code>{v.code}</code> <code>{v.id}</code> {v.measure} <span class="msg">{v.message}</span></li>
          {/each}
        </ul>
      {/if}
    {/if}
    {#if p.state === "pending_review"}
      <div class="row">
        <button type="button" class="button primary" disabled={busy} onclick={() => decide(p, true)}>{$t("city3d.found.approve")}</button>
        <button type="button" class="button" disabled={busy} onclick={() => decide(p, false)}>{$t("city3d.found.reject")}</button>
      </div>
    {/if}
  </li>
{/snippet}

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 0.75rem);
    font-size: var(--text-sm, 0.875rem);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  .title {
    margin: 0;
    font-weight: 600;
    font-size: var(--text-md, 1rem);
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px var(--space-3, 0.75rem);
    margin: 0;
  }
  dt {
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
  }
  dd {
    margin: 0;
  }
  .intent,
  .msg {
    margin: 0;
  }
  .dim,
  .note,
  .when {
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
    font-size: var(--text-caption, 0.8rem);
  }
  .note {
    margin: 0;
  }
  .error {
    margin: 0;
    color: var(--danger, #d70015);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--separator, rgba(127, 127, 127, 0.25));
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent, #0a84ff);
    transition: width 0.4s ease;
  }
  .bar.small {
    height: 4px;
    flex: 1;
  }
  ul.log,
  ul.works,
  ul.steps,
  ul.violations {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  ul.works li {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
  }
  ul.works li > span {
    flex: none;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry {
    padding: var(--space-2, 0.5rem);
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 0.5rem);
    align-items: baseline;
  }
  .state {
    font-size: var(--text-caption, 0.8rem);
    font-weight: 600;
  }
  .state.applied,
  .state.founded,
  .state.work_done {
    color: var(--success, #248a3d);
  }
  .state.rejected,
  .state.refused,
  .state.error,
  .state.stale {
    color: var(--danger, #d70015);
  }
  .state.pending_review {
    color: var(--warning, #b25000);
  }
  .violations li {
    padding-left: var(--space-2, 0.5rem);
    border-left: 2px solid var(--danger, #d70015);
  }
  .violations li.warning {
    border-left-color: var(--warning, #b25000);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 0.5rem);
    flex-wrap: wrap;
  }
  .segmented {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-full, 999px);
    background: var(--separator, rgba(127, 127, 127, 0.18));
  }
  .segmented button {
    border: 0;
    font: inherit;
    font-size: var(--text-xs, 0.75rem);
    padding: 0.2rem 0.7rem;
    border-radius: var(--radius-full, 999px);
    background: transparent;
    color: var(--text, inherit);
    cursor: pointer;
  }
  .segmented button.active {
    background: var(--accent, #0a84ff);
    color: var(--on-accent, #fff);
  }
  .link {
    align-self: flex-start;
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    font-size: var(--text-caption, 0.8rem);
    color: var(--accent, #0a84ff);
    cursor: pointer;
  }
  .steps li {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
