<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type Priority, type Project, type Routine, type Staff } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import CronInput from "$components/company/CronInput.svelte";
  import { CRON_PRESETS, presetOf, relTime } from "$components/company/company-ui";

  const PRIORITIES: Priority[] = ["critical", "high", "medium", "low"];
  const CATCH_UP = ["skip", "once", "all_capped"] as const;
  const CONCURRENCY = ["skip_if_active", "always"] as const;
  const LOCAL_TZ = (() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone;
    } catch {
      return "UTC";
    }
  })();

  let id = $derived(page.params.id ?? "");
  let routines = $state<Routine[]>([]);
  let staff = $state<Staff[]>([]);
  let projects = $state<Project[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let now = $state(Date.now());
  let busyId = $state<string | null>(null);

  type Draft = {
    id: string | null; title: string; body: string; staff_id: string; project_id: string; priority: Priority;
    cron: string | null; tz: string; catch_up: Routine["catch_up"]; concurrency: Routine["concurrency"]; enabled: boolean;
  };
  let form = $state<Draft | null>(null);
  let formBusy = $state(false);
  let formError = $state("");

  async function load(cid: string) {
    try {
      const [r, s, p] = await Promise.all([
        rpc<Routine[]>("routine.list", { company: cid }),
        rpc<Staff[]>("staff.list", { company: cid }),
        rpc<Project[]>("project.list", { company: cid }),
      ]);
      routines = r;
      staff = s;
      projects = p;
      error = "";
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    const cid = id;
    if (!cid) return;
    void load(cid);
    const un = onCompanyChange(cid, () => void load(cid));
    const timer = setInterval(() => (now = Date.now()), 30000);
    return () => {
      clearInterval(timer);
      void un.then((f) => f());
    };
  });

  let alive = $derived(staff.filter((s) => s.status !== "terminated"));
  let staffName = $derived(new Map(staff.map((s) => [s.id, s.name])));

  function cronLabel(cron: string): string {
    const p = presetOf(cron);
    return p ? $t(`company.common.cron.${p}`) : cron;
  }

  function newRoutine() {
    form = {
      id: null, title: "", body: "", staff_id: alive[0]?.id ?? "", project_id: "", priority: "medium",
      cron: CRON_PRESETS.find((p) => p.id === "weekdays")?.cron ?? "0 9 * * 1-5", tz: LOCAL_TZ,
      catch_up: "once", concurrency: "skip_if_active", enabled: true,
    };
    formError = "";
  }

  function editRoutine(r: Routine) {
    form = {
      id: r.id, title: r.title, body: r.body, staff_id: r.staff_id, project_id: r.project_id ?? "", priority: r.priority,
      cron: r.cron, tz: r.tz, catch_up: r.catch_up, concurrency: r.concurrency, enabled: r.enabled,
    };
    formError = "";
  }

  function payload(d: Draft) {
    return {
      title: d.title.trim(), body: d.body, staff_id: d.staff_id, project_id: d.project_id || null, priority: d.priority,
      cron: (d.cron ?? "").trim(), tz: d.tz.trim() || "local", catch_up: d.catch_up, concurrency: d.concurrency, enabled: d.enabled,
    };
  }

  function draftOf(r: Routine): Draft {
    return { ...r, project_id: r.project_id ?? "", id: r.id };
  }

  async function save(e: Event) {
    e.preventDefault();
    const d = form;
    if (!d || !d.title.trim() || !d.staff_id || !d.cron) return;
    formBusy = true;
    formError = "";
    try {
      if (d.id) await rpc("routine.update", { company: id, routine_id: d.id, routine: payload(d) });
      else await rpc("routine.create", { company: id, routine: payload(d) });
      form = null;
      void load(id);
    } catch (err) {
      formError = errorText(err);
    } finally {
      formBusy = false;
    }
  }

  async function act(r: Routine, fn: () => Promise<unknown>, ok?: string) {
    busyId = r.id;
    try {
      await fn();
      if (ok) showToast("success", ok);
      void load(id);
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busyId = null;
    }
  }

  function toggle(r: Routine) {
    void act(r, () => rpc("routine.update", { company: id, routine_id: r.id, routine: payload({ ...draftOf(r), enabled: !r.enabled }) }));
  }

  function fire(r: Routine) {
    void act(r, () => rpc("routine.fire", { company: id, routine_id: r.id }), $t("company.routines.fired", { title: r.title }));
  }

  function remove(r: Routine) {
    if (!confirm($t("company.routines.delete_confirm", { title: r.title }) as string)) return;
    void act(r, () => rpc("routine.delete", { company: id, routine_id: r.id }));
  }

  const WEEK_TOKEN = "\\{\\{week\\}\\}";
  const DATE_TOKEN = "\\{\\{date\\}\\}";
</script>

<div class="page page-wide routines-page">
  <header class="page-head">
    <div>
      <h2 class="page-title">{$t("company.routines.title")}</h2>
      <p class="page-lede">{$t("company.routines.lede")}</p>
    </div>
    {#if routines.length}<button type="button" class="button active" disabled={alive.length === 0} onclick={newRoutine}>{$t("company.routines.new")}</button>{/if}
  </header>

  {#if error}<p class="field-error">{error}</p>{/if}

  {#if loaded && routines.length === 0}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/hourglass_not_done.png" alt="" width="72" height="72" draggable="false" />
      <div class="empty-state-title">{$t("company.routines.empty_title")}</div>
      <p class="empty-state-hint">{alive.length ? $t("company.routines.empty_hint") : $t("company.routines.need_staff")}</p>
      {#if alive.length}
        <button type="button" class="button active" onclick={newRoutine}>{$t("company.routines.new")}</button>
      {:else}
        <a class="button active" href="/llm/company/{id}/team">{$t("company.routines.go_team")}</a>
      {/if}
    </div>
  {:else if routines.length}
    <div class="group">
      {#each routines as r (r.id)}
        <div class="group-row routine-row" class:off={!r.enabled}>
          <button type="button" class="toggle" role="switch" aria-checked={r.enabled} aria-label={$t("company.routines.enabled")} disabled={busyId === r.id} onclick={() => toggle(r)}><span class="toggle-knob"></span></button>
          <span class="group-row-content">
            <span class="group-row-title">{r.title}</span>
            <span class="group-row-sub">
              {staffName.get(r.staff_id) ?? "—"} · {cronLabel(r.cron)}{r.tz !== "local" ? ` (${r.tz})` : ""} · {$t(`company.common.priority.${r.priority}`)}
            </span>
          </span>
          <span class="col">
            <span class="col-label">{$t("company.routines.next")}</span>
            <span class="col-val">{r.enabled && r.next_run_ms ? relTime(r.next_run_ms, $t, now) : "—"}</span>
          </span>
          <span class="col">
            <span class="col-label">{$t("company.routines.last")}</span>
            <span class="col-val">{relTime(r.last_run_ms, $t, now)}</span>
          </span>
          <span class="group-row-trailing">
            <button type="button" class="button" disabled={busyId === r.id} onclick={() => fire(r)}>{$t("company.routines.fire")}</button>
            <button type="button" class="button" onclick={() => editRoutine(r)}>{$t("company.common.edit")}</button>
            <button type="button" class="icon-btn" aria-label={$t("company.common.delete")} disabled={busyId === r.id} onclick={() => remove(r)}>×</button>
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if form}
  <div class="dialog-backdrop" role="presentation" onkeydown={(e) => e.key === "Escape" && (form = null)}>
    <div class="dialog-card wide" role="dialog" aria-modal="true" aria-labelledby="routine-form-title">
      <form class="dialog-form" onsubmit={save}>
        <h2 class="dialog-title" id="routine-form-title">{form.id ? $t("company.routines.edit_title") : $t("company.routines.new_title")}</h2>
        <label class="field">
          <span class="field-label">{$t("company.routines.form.title")}</span>
          <input class="input" type="text" bind:value={form.title} placeholder={$t("company.routines.form.title_placeholder", { week: WEEK_TOKEN })} required />
          <span class="field-hint">{$t("company.routines.form.title_hint", { date: DATE_TOKEN, week: WEEK_TOKEN })}</span>
        </label>
        <label class="field">
          <span class="field-label">{$t("company.routines.form.body")}</span>
          <textarea class="input" rows="3" bind:value={form.body}></textarea>
        </label>
        <div class="row">
          <label class="field grow">
            <span class="field-label">{$t("company.routines.form.assignee")}</span>
            <select class="input" bind:value={form.staff_id} required>
              {#each alive as s (s.id)}<option value={s.id}>{s.name} · {s.title || s.role}</option>{/each}
            </select>
          </label>
          <label class="field grow">
            <span class="field-label">{$t("company.routines.form.project")}</span>
            <select class="input" bind:value={form.project_id}>
              <option value="">{$t("company.common.none")}</option>
              {#each projects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{$t("company.routines.form.priority")}</span>
            <select class="input" bind:value={form.priority}>
              {#each PRIORITIES as p (p)}<option value={p}>{$t(`company.common.priority.${p}`)}</option>{/each}
            </select>
          </label>
        </div>
        <div class="row">
          <div class="field grow">
            <span class="field-label">{$t("company.routines.form.schedule")}</span>
            <CronInput bind:value={form.cron} allowNone={false} />
          </div>
          <label class="field">
            <span class="field-label">{$t("company.routines.form.tz")}</span>
            <input class="input tz" type="text" list="tz-list" bind:value={form.tz} spellcheck="false" />
            <datalist id="tz-list">
              <option value="local"></option>
              <option value={LOCAL_TZ}></option>
              <option value="UTC"></option>
            </datalist>
          </label>
        </div>
        <div class="row">
          <label class="field grow">
            <span class="field-label">{$t("company.routines.form.catch_up")}</span>
            <select class="input" bind:value={form.catch_up}>
              {#each CATCH_UP as c (c)}<option value={c}>{$t(`company.routines.catch_up.${c}`)}</option>{/each}
            </select>
          </label>
          <label class="field grow">
            <span class="field-label">{$t("company.routines.form.concurrency")}</span>
            <select class="input" bind:value={form.concurrency}>
              {#each CONCURRENCY as c (c)}<option value={c}>{$t(`company.routines.concurrency.${c}`)}</option>{/each}
            </select>
          </label>
        </div>
        {#if formError}<p class="field-error">{formError}</p>{/if}
        <div class="dialog-actions">
          <button type="button" class="button" disabled={formBusy} onclick={() => (form = null)}>{$t("company.common.cancel")}</button>
          <button type="submit" class="button active" disabled={formBusy || !form.title.trim() || !form.staff_id || !form.cron}>{$t("company.common.save")}</button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .routines-page { display: flex; flex-direction: column; gap: var(--space-4); }
  .routines-page .page-head { margin-bottom: 0; }
  .routines-page .page-title { font-size: var(--text-lg); }
  .routine-row.off .group-row-title { color: var(--text-dim); }
  .col { display: flex; flex-direction: column; min-width: 90px; flex-shrink: 0; }
  .col-label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-faint); }
  .col-val { font-size: var(--text-xs); color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .icon-btn { background: none; border: 0; font-size: 18px; line-height: 1; color: var(--text-dim); cursor: pointer; padding: 2px 6px; border-radius: var(--radius-sm); }
  .icon-btn:hover { background: var(--fill-2); color: var(--red); }
  .dialog-form { display: flex; flex-direction: column; gap: var(--space-3); }
  .wide { width: min(600px, 100%); max-height: calc(100vh - 64px); overflow-y: auto; }
  .row { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: flex-start; }
  .grow { flex: 1; min-width: 180px; }
  .tz { width: 180px; font-family: var(--font-mono); }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  @media (max-width: 720px) { .col { display: none; } }
</style>
