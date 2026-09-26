<script lang="ts">
  /**
   * Procedural learning of one bot: how it does its job, kept apart from
   * memory (what it knows about you). Feedback becomes observations; a
   * candidate is a text overlay on a skill, evaluated on dev and holdout
   * cases against the current behaviour before it can be promoted. Nothing
   * is promoted on a model's word; shared procedures are never promoted
   * automatically. Collapsed by default and loaded on first open.
   */
  import { t } from "$lib/i18n";
  import {
    candidateTone, errorText, observationLabelKeys, promotableRun, shortTime, isAbsolutePath,
    procedureVersionParams, isActiveCandidate,
    type CandidateState, type EvalRunLite,
  } from "$lib/stores/assist-missions-store.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";

  let { botId }: { botId: string } = $props();

  interface Policy { min_dev_cases: number; min_holdout_cases: number; min_dev_gain: number; no_holdout_regression: boolean; unknown_blocks: boolean }
  interface Settings { bot_id: string; enabled: boolean; auto_promote_local: boolean; policy: Policy }
  interface Observation { id: string; skill: string; task_ref: string; source: string; kind: string; text: string; polarity: number; outcome: string; dup_of: string | null; revoked_ms: number | null; revoked_reason: string | null; created_ms: number }
  interface Candidate { id: string; skill: string; scope: string; kind: string; title: string; state: CandidateState; current_version: number; reason: string | null }
  interface Active { bot_id: string; skill: string; scope: string; candidate_id: string; version: number; overlay: string; since_ms: number }
  interface Promotion { id: string; action: string; candidate_id: string | null; version: number | null; by_whom: string; reason: string; created_ms: number }
  type Check =
    | { kind: "contains"; text: string } | { kind: "excludes"; text: string } | { kind: "regex"; pattern: string }
    | { kind: "count_lines"; pattern: string; min: number; max: number } | { kind: "no_terms"; terms: string[] }
    | { kind: "json_keys"; keys: string[] } | { kind: "max_chars"; max: number };
  interface EvalCase { id: string; skill: string; split: string; name: string; input: string; checks: Check[] }
  interface Split { n: number; baseline_pass: number; candidate_pass: number; unknown: number; regressions: number }
  interface EvalRun extends EvalRunLite { mode: string; baseline: string; results: { name: string; split: string; baseline: string; candidate: string; baseline_notes: string[]; candidate_notes: string[] }[]; dev: Split; holdout: Split; reasons: string[] }
  interface RunningEval { run_id: string; candidate_id: string; candidate_version: number; state: string }
  interface Overview { settings: Settings; observations: Observation[]; candidates: Candidate[]; active: Active[]; promotions: Promotion[]; cases: Record<string, EvalCase[]>; running_evals?: RunningEval[] }
  interface EvalStart { run_id: string; state: "running" | "done"; already_running?: boolean }
  interface EvalStatus { run_id: string; state: "running" | "done" | "failed"; error?: string | null }
  interface CandidateView { candidate: Candidate; versions: { version: number; overlay: string; diff: string; reason: string; observations: string[] }[]; runs: EvalRun[] }

  const CHECK_KINDS = ["contains", "excludes", "regex", "count_lines", "no_terms", "json_keys", "max_chars"] as const;

  let open = $state(false);
  let data = $state<Overview | null>(null);
  let settings = $state<Settings | null>(null);
  let busy = $state(false);
  let error = $state("");
  let exportPath = $state("");
  let notice = $state<string | null>(null);
  let confirming = $state<{ label: string; run: () => Promise<unknown> } | null>(null);
  let openCandidate = $state<string | null>(null);
  let candidateView = $state<CandidateView | null>(null);

  // Feedback form
  let obSkill = $state("");
  let obTask = $state("");
  let obText = $state("");
  let obPolarity = $state<-1 | 0 | 1>(-1);
  let obKind = $state<"procedure" | "preference">("procedure");
  let revokeReason = $state<Record<string, string>>({});

  // Proposal form
  let prSkill = $state("");
  let prTitle = $state("");
  let prOverlay = $state("");
  let prReason = $state("");
  let prObs = $state<Record<string, boolean>>({});

  // Case form
  let caSkill = $state("");
  let caSplit = $state<"dev" | "holdout">("dev");
  let caName = $state("");
  let caInput = $state("");
  let caChecks = $state<{ kind: (typeof CHECK_KINDS)[number]; a: string; min: string; max: string }[]>([{ kind: "contains", a: "", min: "", max: "" }]);

  let rollbackReason = $state<Record<string, string>>({});

  let skills = $derived.by(() => {
    const s = new Set<string>();
    for (const o of data?.observations ?? []) s.add(o.skill);
    for (const c of data?.candidates ?? []) s.add(c.skill);
    for (const k of Object.keys(data?.cases ?? {})) s.add(k);
    for (const a of data?.active ?? []) s.add(a.skill);
    return [...s].filter(Boolean).sort();
  });
  let proposable = $derived((data?.observations ?? []).filter((o) => !o.revoked_ms && (!prSkill || o.skill === prSkill)));

  async function load() {
    try {
      const d = await invoke<Overview>("assist_learning_overview", { botId });
      data = d;
      settings = structuredClone(d.settings);
      error = "";
      for (const r of d.running_evals ?? []) watchRun(r.run_id);
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    void botId;
    if (open) void load();
  });

  async function act(fn: () => Promise<unknown>, done?: string) {
    if (busy) return;
    busy = true;
    error = "";
    notice = null;
    confirming = null;
    try {
      await fn();
      if (done) notice = done;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
    await load();
    if (openCandidate) await loadCandidate(openCandidate);
  }

  async function loadCandidate(id: string) {
    try {
      candidateView = await invoke<CandidateView>("assist_learning_candidate", { candidateId: id });
    } catch (e) {
      error = errorText(e);
    }
  }

  function toggleCandidate(id: string) {
    if (openCandidate === id) {
      openCandidate = null;
      candidateView = null;
      return;
    }
    openCandidate = id;
    candidateView = null;
    void loadCandidate(id);
  }

  function saveSettings() {
    if (!settings) return;
    const s = $state.snapshot(settings);
    void act(() => invoke("assist_learning_settings_save", { settings: { ...s, bot_id: botId } }), $t("assist.learning.saved"));
  }

  function observe() {
    if (!obSkill.trim() || !obText.trim()) return;
    const observation = { bot_id: botId, skill: obSkill.trim(), task_ref: obTask.trim() || "manual", text: obText.trim(), polarity: obPolarity, kind: obKind };
    void act(async () => {
      await invoke("assist_learning_observe", { observation });
      obText = "";
      obTask = "";
    });
  }

  function propose() {
    if (!prSkill.trim() || !prTitle.trim() || !prOverlay.trim()) return;
    const candidate = {
      bot_id: botId, skill: prSkill.trim(), title: prTitle.trim(), overlay: prOverlay, reason: prReason.trim(),
      observations: Object.entries(prObs).filter(([, v]) => v).map(([k]) => k),
    };
    void act(async () => {
      await invoke("assist_learning_propose", { candidate });
      prTitle = ""; prOverlay = ""; prReason = ""; prObs = {};
    });
  }

  function buildChecks(): Check[] | null {
    const out: Check[] = [];
    for (const c of caChecks) {
      const a = c.a.trim();
      const list = a.split("\n").map((x) => x.trim()).filter(Boolean);
      switch (c.kind) {
        case "contains": case "excludes": if (!a) return null; out.push({ kind: c.kind, text: a }); break;
        case "regex": if (!a) return null; out.push({ kind: "regex", pattern: a }); break;
        case "count_lines": {
          const min = Number(c.min), max = Number(c.max);
          if (!a || !Number.isInteger(min) || !Number.isInteger(max) || min < 0 || max < min) return null;
          out.push({ kind: "count_lines", pattern: a, min, max }); break;
        }
        case "no_terms": if (!list.length) return null; out.push({ kind: "no_terms", terms: list }); break;
        case "json_keys": if (!list.length) return null; out.push({ kind: "json_keys", keys: list }); break;
        case "max_chars": { const max = Number(c.max); if (!Number.isInteger(max) || max <= 0) return null; out.push({ kind: "max_chars", max }); break; }
      }
    }
    return out.length ? out : null;
  }

  function addCase() {
    const checks = buildChecks();
    if (!caSkill.trim() || !caName.trim() || !caInput.trim() || !checks) {
      error = $t("assist.learning.case_invalid");
      return;
    }
    const c = { bot_id: botId, skill: caSkill.trim(), split: caSplit, name: caName.trim(), input: caInput, checks };
    void act(async () => {
      await invoke("assist_learning_add_case", { case: c });
      caName = ""; caInput = ""; caChecks = [{ kind: "contains", a: "", min: "", max: "" }];
    });
  }

  function checkText(c: Check): string {
    switch (c.kind) {
      case "contains": case "excludes": return `${c.kind} "${c.text}"`;
      case "regex": return `regex /${c.pattern}/`;
      case "count_lines": return `count_lines /${c.pattern}/ ${c.min}–${c.max}`;
      case "no_terms": return `no_terms ${c.terms.join(", ")}`;
      case "json_keys": return `json_keys ${c.keys.join(", ")}`;
      case "max_chars": return `max_chars ${c.max}`;
    }
  }

  // A live evaluation runs in the background (it can take many minutes, more
  // than an IPC request stays open); the screen follows it by polling.
  const watching = new Set<string>();
  let gone = false;
  onDestroy(() => (gone = true));

  function watchRun(runId: string) {
    if (watching.has(runId)) return;
    watching.add(runId);
    const tick = async () => {
      if (gone) return;
      let st: EvalStatus | null = null;
      try {
        st = await invoke<EvalStatus>("assist_learning_eval_status", { runId });
      } catch {
        st = null;
      }
      if (st?.state === "running") {
        setTimeout(tick, 3000);
        return;
      }
      watching.delete(runId);
      if (gone) return;
      if (st?.state === "failed") error = $t("assist.learning.eval_failed", { error: st.error ?? "—" });
      else if (st) notice = $t("assist.learning.eval_done");
      await load();
      if (openCandidate) await loadCandidate(openCandidate);
    };
    setTimeout(tick, 3000);
  }

  function evaluate(id: string, mode: "offline" | "live") {
    const run = async () => {
      const r = await invoke<EvalStart>("assist_learning_evaluate", { input: { candidate_id: id, mode } });
      if (r.state === "running") {
        notice = $t(r.already_running ? "assist.learning.eval_already_running" : "assist.learning.eval_started");
        watchRun(r.run_id);
      }
    };
    if (mode === "live") confirming = { label: $t("assist.learning.confirm_live"), run };
    else void act(run);
  }

  async function exportAll() {
    let path: string | null = null;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const r = await save({ defaultPath: `learning-${botId}.json`, filters: [{ name: "JSON", extensions: ["json"] }] });
      path = typeof r === "string" ? r : null;
    } catch {
      error = $t("assist.learning.picker_failed");
      return;
    }
    if (!path) return;
    const p = path;
    void act(() => invoke("assist_learning_export", { botId, path: p }), $t("assist.learning.exported", { path: p }));
  }

  /** The same export to a path typed by hand (no native dialog): a `.json` file or a folder. */
  function exportTyped() {
    const typed = exportPath.trim();
    if (!isAbsolutePath(typed)) {
      error = $t("mission.path.err_absolute");
      return;
    }
    const p = /\.json$/i.test(typed) ? typed : `${typed.replace(/[\\/]+$/, "")}/learning-${botId}.json`;
    void act(() => invoke("assist_learning_export", { botId, path: p }).then(() => (exportPath = "")), $t("assist.learning.exported", { path: p }));
  }

  function forget() {
    confirming = { label: $t("assist.learning.confirm_forget"), run: () => invoke("assist_learning_forget", { botId }) };
  }
</script>

<details class="learning" bind:open>
  <summary>{$t("assist.learning.title")}</summary>
  <div class="inner" aria-busy={busy}>
    <p class="hint">{$t("assist.learning.lede")} <a class="link" href="/llm/memory">{$t("assist.learning.memory_link")}</a></p>
    <ul class="rules">
      <li>{$t("assist.learning.rule_shared_off")}</li>
      <li>{$t("assist.learning.rule_optin")}</li>
      <li>{$t("assist.learning.rule_live_cost")}</li>
      <li>{$t("assist.learning.rule_model")}</li>
    </ul>

    {#if confirming}
      <div class="notice confirm" role="alertdialog" aria-label={confirming.label}>
        <span>{confirming.label}</span>
        <div class="actions">
          <button type="button" class="button active" disabled={busy} onclick={() => { const c = confirming; if (c) void act(c.run); }}>{$t("mission.confirm.yes")}</button>
          <button type="button" class="button" onclick={() => (confirming = null)}>{$t("mission.cancel")}</button>
        </div>
      </div>
    {/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if !data || !settings}
      {#if !error}<p class="hint" role="status">{$t("assist.learning.loading")}</p>{/if}
    {:else}
      <section class="sec">
        <h4>{$t("assist.learning.settings")}</h4>
        <label class="check"><input type="checkbox" bind:checked={settings.enabled} disabled={busy} /> <span>{$t("assist.learning.enabled")}</span></label>
        <label class="check"><input type="checkbox" bind:checked={settings.auto_promote_local} disabled={busy} /> <span>{$t("assist.learning.auto_promote_local")}</span></label>
        <p class="hint">{$t("assist.learning.auto_promote_hint")}</p>
        <details class="adv">
          <summary>{$t("assist.learning.policy")}</summary>
          <div class="grid">
            <label class="field"><span class="field-label">{$t("assist.learning.min_dev_cases")}</span><input class="input" type="number" min="0" bind:value={settings.policy.min_dev_cases} /></label>
            <label class="field"><span class="field-label">{$t("assist.learning.min_holdout_cases")}</span><input class="input" type="number" min="0" bind:value={settings.policy.min_holdout_cases} /></label>
            <label class="field"><span class="field-label">{$t("assist.learning.min_dev_gain")}</span><input class="input" type="number" bind:value={settings.policy.min_dev_gain} /></label>
          </div>
          <label class="check"><input type="checkbox" bind:checked={settings.policy.no_holdout_regression} /> <span>{$t("assist.learning.no_holdout_regression")}</span></label>
          <label class="check"><input type="checkbox" bind:checked={settings.policy.unknown_blocks} /> <span>{$t("assist.learning.unknown_blocks")}</span></label>
        </details>
        <div class="actions"><button type="button" class="button" disabled={busy} onclick={saveSettings}>{$t("assist.learning.save")}</button></div>
      </section>

      <section class="sec">
        <h4>{$t("assist.learning.active_title")}</h4>
        {#if data.active.length === 0}
          <p class="hint">{$t("assist.learning.active_empty")}</p>
        {:else}
          <ul class="plain">
            {#each data.active as a (a.skill + a.scope)}
              <li>
                <div class="line">
                  <span class="pill green">{$t("assist.learning.active_badge")}</span>
                  <strong>{a.skill}</strong> · {$t("assist.learning.procedure_version", procedureVersionParams(data.candidates, a.candidate_id, a.version))} · {$t("assist.learning.since", { when: shortTime(a.since_ms) })}
                </div>
                <details class="adv"><summary>{$t("assist.learning.overlay")}</summary><pre class="pre">{a.overlay}</pre></details>
                <div class="actions">
                  <input class="input grow" type="text" placeholder={$t("assist.learning.reason")} aria-label={$t("assist.learning.reason")}
                    value={rollbackReason[a.skill] ?? ""} oninput={(e) => (rollbackReason[a.skill] = e.currentTarget.value)} />
                  <button type="button" class="button" disabled={busy || !(rollbackReason[a.skill] ?? "").trim()}
                    onclick={() => act(() => invoke("assist_learning_rollback", { botId, skill: a.skill, scope: a.scope, reason: rollbackReason[a.skill].trim() }))}>{$t("assist.learning.rollback")}</button>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="sec">
        <h4>{$t("assist.learning.observations_title")}</h4>
        <p class="hint">{$t("assist.learning.observations_hint")}</p>
        {#if data.observations.length === 0}
          <p class="hint">{$t("assist.learning.observations_empty")}</p>
        {:else}
          <ul class="plain">
            {#each data.observations as o (o.id)}
              <li class:revoked={!!o.revoked_ms}>
                <div class="line">
                  <span class="tag">{o.skill}</span>
                  <span class="tag">{o.polarity > 0 ? "+" : o.polarity < 0 ? "−" : "·"}</span>
                  {#each observationLabelKeys(o) as k (k)}<span class="tag" class:warn={k.endsWith("hypothesis") || k.endsWith("duplicate")}>{$t(k)}</span>{/each}
                  <span class="dim">{shortTime(o.created_ms)}</span>
                </div>
                <div class="text">{o.text}</div>
                {#if o.revoked_ms}<p class="dim">{$t("assist.learning.revoked_reason", { reason: o.revoked_reason ?? "—" })}</p>
                {:else}
                  <div class="actions">
                    <input class="input grow" type="text" placeholder={$t("assist.learning.reason")} aria-label={$t("assist.learning.reason")}
                      value={revokeReason[o.id] ?? ""} oninput={(e) => (revokeReason[o.id] = e.currentTarget.value)} />
                    <button type="button" class="button small" disabled={busy || !(revokeReason[o.id] ?? "").trim()}
                      onclick={() => act(() => invoke("assist_learning_revoke", { observationId: o.id, reason: revokeReason[o.id].trim() }))}>{$t("assist.learning.revoke")}</button>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <details class="adv">
          <summary>{$t("assist.learning.add_feedback")}</summary>
          <div class="form">
            <div class="grid">
              <label class="field"><span class="field-label">{$t("assist.learning.skill")}</span><input class="input" list="learning-skills-{botId}" bind:value={obSkill} /></label>
              <label class="field"><span class="field-label">{$t("assist.learning.task_ref")}</span><input class="input" bind:value={obTask} /></label>
              <label class="field"><span class="field-label">{$t("assist.learning.polarity")}</span>
                <select class="input" bind:value={obPolarity}>
                  <option value={-1}>{$t("assist.learning.polarity_neg")}</option>
                  <option value={0}>{$t("assist.learning.polarity_neutral")}</option>
                  <option value={1}>{$t("assist.learning.polarity_pos")}</option>
                </select>
              </label>
              <label class="field"><span class="field-label">{$t("assist.learning.kind")}</span>
                <select class="input" bind:value={obKind}>
                  <option value="procedure">{$t("assist.learning.kind_procedure")}</option>
                  <option value="preference">{$t("assist.learning.kind_preference")}</option>
                </select>
              </label>
            </div>
            {#if obKind === "preference"}<p class="hint">{$t("assist.learning.preference_hint")} <a class="link" href="/llm/memory">{$t("assist.learning.memory_link")}</a></p>{/if}
            <label class="field"><span class="field-label">{$t("assist.learning.text")}</span><textarea class="input" rows="2" bind:value={obText}></textarea></label>
            <div class="actions"><button type="button" class="button" disabled={busy || !obSkill.trim() || !obText.trim()} onclick={observe}>{$t("assist.learning.observe")}</button></div>
          </div>
        </details>
      </section>

      <section class="sec">
        <h4>{$t("assist.learning.candidates_title")}</h4>
        {#if data.candidates.length === 0}
          <p class="hint">{$t("assist.learning.candidates_empty")}</p>
        {:else}
          <ul class="plain">
            {#each data.candidates as c (c.id)}
              <li>
                <button type="button" class="row-head" aria-expanded={openCandidate === c.id} onclick={() => toggleCandidate(c.id)}>
                  <span class="pill {candidateTone(c.state)}">{$t(`assist.learning.state.${c.state}`)}</span>
                  <strong>{c.title}</strong>
                  <span class="tag">{c.skill}</span>
                  <span class="dim">v{c.current_version} · {c.id.slice(0, 8)}</span>
                  {#if isActiveCandidate(data.active, c) && c.state !== "active"}<span class="pill green">{$t("assist.learning.active_badge")}</span>{/if}
                  {#if (data.running_evals ?? []).some((r) => r.candidate_id === c.id)}<span class="tag">{$t("assist.learning.eval_running")}</span>{/if}
                  {#if c.kind === "executable"}<span class="tag warn">{$t("assist.learning.executable_unsupported")}</span>{/if}
                </button>
                {#if openCandidate === c.id}
                  {#if !candidateView}
                    <p class="hint">{$t("assist.learning.loading")}</p>
                  {:else}
                    {@const eligible = promotableRun(candidateView.candidate.current_version, candidateView.runs)}
                    {@const cur = candidateView.versions.find((v) => v.version === candidateView!.candidate.current_version)}
                    <div class="cand">
                      {#if cur}
                        {#if cur.reason}<p class="hint">{cur.reason}</p>{/if}
                        <details class="adv" open><summary>{$t("assist.learning.diff")}</summary><pre class="pre">{cur.diff || cur.overlay}</pre></details>
                      {/if}
                      <div class="actions">
                        <button type="button" class="button" disabled={busy} onclick={() => evaluate(c.id, "offline")}>{$t("assist.learning.eval_offline")}</button>
                        <button type="button" class="button" disabled={busy} onclick={() => evaluate(c.id, "live")}>{$t("assist.learning.eval_live")}</button>
                        <button type="button" class="button active" disabled={busy || !eligible || c.state === "active"}
                          title={eligible ? "" : $t("assist.learning.promote_needs_eval")}
                          onclick={() => { if (eligible) void act(() => invoke("assist_learning_promote", { candidateId: c.id, evalRunId: eligible.id })); }}>{$t("assist.learning.promote")}</button>
                      </div>
                      {#if !eligible}<p class="hint">{$t("assist.learning.promote_needs_eval")}</p>{/if}
                      {#each [...candidateView.runs].sort((a, b) => (b.created_ms ?? 0) - (a.created_ms ?? 0)) as r (r.id)}
                        <details class="adv run">
                          <summary>
                            <span class="pill {r.decision === 'eligible' ? 'green' : r.decision === 'rejected' ? 'red' : 'grey'}">{$t(`assist.learning.decision.${r.decision}`)}</span>
                            v{r.candidate_version} · {r.mode} · {shortTime(r.created_ms)}
                            · {$t("assist.learning.split_line", { split: "dev", baseline: r.dev.baseline_pass, candidate: r.dev.candidate_pass, count: r.dev.n })}
                            · {$t("assist.learning.split_line", { split: "holdout", baseline: r.holdout.baseline_pass, candidate: r.holdout.candidate_pass, count: r.holdout.n })}
                          </summary>
                          {#if r.reasons.length}<ul class="rules">{#each r.reasons as why, i (i)}<li>{why}</li>{/each}</ul>{/if}
                          <table class="results">
                            <thead><tr><th>{$t("assist.learning.case")}</th><th>{$t("assist.learning.split")}</th><th>{$t("assist.learning.baseline")}</th><th>{$t("assist.learning.candidate")}</th></tr></thead>
                            <tbody>
                              {#each r.results as res, i (i)}
                                <tr><td>{res.name}</td><td>{res.split}</td><td class={res.baseline}>{res.baseline}</td><td class={res.candidate}>{res.candidate}</td></tr>
                              {/each}
                            </tbody>
                          </table>
                        </details>
                      {/each}
                      {#if candidateView.versions.length > 1}
                        <details class="adv"><summary>{$t("assist.learning.versions", { count: candidateView.versions.length })}</summary>
                          <ul class="plain">{#each candidateView.versions as v (v.version)}<li>v{v.version} · {v.reason}</li>{/each}</ul>
                        </details>
                      {/if}
                    </div>
                  {/if}
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <details class="adv">
          <summary>{$t("assist.learning.propose")}</summary>
          <div class="form">
            <div class="grid">
              <label class="field"><span class="field-label">{$t("assist.learning.skill")}</span><input class="input" list="learning-skills-{botId}" bind:value={prSkill} /></label>
              <label class="field"><span class="field-label">{$t("assist.learning.candidate_title")}</span><input class="input" bind:value={prTitle} /></label>
            </div>
            <label class="field"><span class="field-label">{$t("assist.learning.overlay")}</span><textarea class="input mono" rows="4" bind:value={prOverlay}></textarea></label>
            <label class="field"><span class="field-label">{$t("assist.learning.reason")}</span><input class="input" bind:value={prReason} /></label>
            {#if proposable.length}
              <div class="field-label">{$t("assist.learning.based_on")}</div>
              {#each proposable as o (o.id)}
                <label class="check"><input type="checkbox" checked={!!prObs[o.id]} onchange={(e) => (prObs[o.id] = e.currentTarget.checked)} /> <span class="clip">{o.text}</span></label>
              {/each}
            {/if}
            <div class="actions"><button type="button" class="button" disabled={busy || !prSkill.trim() || !prTitle.trim() || !prOverlay.trim()} onclick={propose}>{$t("assist.learning.propose_submit")}</button></div>
          </div>
        </details>
      </section>

      <section class="sec">
        <h4>{$t("assist.learning.cases_title")}</h4>
        <p class="hint">{$t("assist.learning.cases_hint")}</p>
        {#each Object.entries(data.cases) as [skill, cases] (skill)}
          <div class="field-label">{skill}</div>
          <ul class="plain">
            {#each cases as ec (ec.id)}
              <li>
                <div class="line">
                  <span class="tag">{ec.split}</span><strong>{ec.name}</strong>
                  <span class="dim">{ec.checks.map(checkText).join(" · ")}</span>
                  <button type="button" class="button small" disabled={busy} onclick={() => act(() => invoke("assist_learning_delete_case", { caseId: ec.id }))}>{$t("assist.learning.delete")}</button>
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="hint">{$t("assist.learning.cases_empty")}</p>
        {/each}
        <details class="adv">
          <summary>{$t("assist.learning.add_case")}</summary>
          <div class="form">
            <div class="grid">
              <label class="field"><span class="field-label">{$t("assist.learning.skill")}</span><input class="input" list="learning-skills-{botId}" bind:value={caSkill} /></label>
              <label class="field"><span class="field-label">{$t("assist.learning.split")}</span>
                <select class="input" bind:value={caSplit}><option value="dev">dev</option><option value="holdout">holdout</option></select>
              </label>
              <label class="field"><span class="field-label">{$t("assist.learning.case_name")}</span><input class="input" bind:value={caName} /></label>
            </div>
            <label class="field"><span class="field-label">{$t("assist.learning.case_input")}</span><textarea class="input" rows="3" bind:value={caInput}></textarea></label>
            <div class="field-label">{$t("assist.learning.checks")}</div>
            {#each caChecks as ck, i (i)}
              <div class="actions">
                <select class="input narrow" bind:value={ck.kind} aria-label={$t("assist.learning.checks")}>
                  {#each CHECK_KINDS as k (k)}<option value={k}>{$t(`assist.learning.check.${k}`)}</option>{/each}
                </select>
                {#if ck.kind === "no_terms" || ck.kind === "json_keys"}
                  <textarea class="input grow" rows="2" bind:value={ck.a} placeholder={$t("assist.learning.one_per_line")}></textarea>
                {:else if ck.kind !== "max_chars"}
                  <input class="input grow" type="text" bind:value={ck.a} placeholder={ck.kind === "contains" || ck.kind === "excludes" ? $t("assist.learning.check_text") : $t("assist.learning.check_pattern")} />
                {/if}
                {#if ck.kind === "count_lines"}
                  <input class="input tiny" type="number" min="0" bind:value={ck.min} placeholder="min" aria-label="min" />
                {/if}
                {#if ck.kind === "count_lines" || ck.kind === "max_chars"}
                  <input class="input tiny" type="number" min="0" bind:value={ck.max} placeholder="max" aria-label="max" />
                {/if}
                <button type="button" class="button small" onclick={() => (caChecks = caChecks.filter((_, j) => j !== i))} aria-label={$t("assist.learning.delete")}>×</button>
              </div>
            {/each}
            <div class="actions">
              <button type="button" class="button small" onclick={() => (caChecks = [...caChecks, { kind: "contains", a: "", min: "", max: "" }])}>{$t("assist.learning.add_check")}</button>
              <button type="button" class="button" disabled={busy} onclick={addCase}>{$t("assist.learning.add_case_submit")}</button>
            </div>
          </div>
        </details>
      </section>

      {#if data.promotions.length}
        <details class="adv">
          <summary>{$t("assist.learning.history")}</summary>
          <ul class="plain">
            {#each data.promotions as p (p.id)}<li class="dim">{`${shortTime(p.created_ms)} · ${p.action} ${$t("assist.learning.procedure_version", procedureVersionParams(data.candidates, p.candidate_id, p.version))} · ${p.by_whom} · ${p.reason}`}</li>{/each}
          </ul>
        </details>
      {/if}

      <div class="actions">
        <button type="button" class="button" disabled={busy} onclick={exportAll}>{$t("assist.learning.export")}</button>
        <button type="button" class="button" disabled={busy} onclick={forget}>{$t("assist.learning.forget")}</button>
      </div>
      <form class="actions" onsubmit={(e) => { e.preventDefault(); exportTyped(); }}>
        <input class="input grow mono" type="text" bind:value={exportPath} placeholder={$t("assist.learning.export_placeholder")} aria-label={$t("assist.learning.export_typed_label")} autocomplete="off" spellcheck={false} />
        <button type="submit" class="button" disabled={busy || !exportPath.trim()}>{$t("mission.path.export_submit")}</button>
      </form>

      <datalist id="learning-skills-{botId}">{#each skills as s (s)}<option value={s}></option>{/each}</datalist>
    {/if}
  </div>
</details>

<style>
  .learning > summary { cursor: pointer; font-weight: 600; font-size: 15px; }
  .inner { display: flex; flex-direction: column; gap: 12px; margin-top: 12px; }
  h4 { margin: 0; font-size: 14px; }
  .sec { display: flex; flex-direction: column; gap: 8px; padding-top: 10px; border-top: 1px solid var(--separator); }
  .hint { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .dim { margin: 0; font-size: 12px; color: var(--text-dim); }
  .rules { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-muted); display: flex; flex-direction: column; gap: 2px; }
  .notice { margin: 0; padding: 8px 12px; border-radius: var(--radius-sm); background: var(--fill-2); font-size: 13px; display: flex; flex-direction: column; gap: 8px; }
  .notice.confirm { background: var(--accent-soft); }
  .error { color: var(--error, #b42318); margin: 0; overflow-wrap: anywhere; font-size: 13px; }
  .check { display: inline-flex; gap: 8px; align-items: center; font-size: 13px; }
  .field { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 8px; }
  .form { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
  .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .grow { flex: 1 1 180px; min-width: 0; }
  .narrow { max-width: 200px; }
  .tiny { width: 80px; }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .mono, textarea.mono { font-family: var(--font-mono); }
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .line { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 13px; }
  .text { font-size: 13px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .revoked .text { text-decoration: line-through; color: var(--text-dim); }
  .clip { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 520px; }
  .row-head { display: flex; flex-wrap: wrap; gap: 6px 8px; align-items: center; width: 100%; padding: 4px 0; background: none; border: 0; color: inherit; cursor: pointer; text-align: left; font: inherit; font-size: 13px; }
  .cand { display: flex; flex-direction: column; gap: 8px; padding: 8px 0 8px 12px; border-left: 2px solid var(--separator); }
  .tag { padding: 0 6px; border-radius: 999px; font-size: 12px; color: var(--text-muted); background: var(--fill-2); }
  .tag.warn { color: var(--orange); background: color-mix(in srgb, var(--orange) 14%, transparent); }
  .pill { display: inline-flex; padding: 1px 8px; border-radius: 999px; font-size: 12px; font-weight: 600; color: var(--text-muted); background: var(--fill-2); }
  .pill.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .pill.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .pill.green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .pill.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .pre { margin: 4px 0 0; padding: 8px; max-height: 240px; overflow: auto; border-radius: var(--radius-sm); background: var(--fill-2); font-family: var(--font-mono); font-size: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .adv summary { cursor: pointer; font-size: 13px; color: var(--accent-text); }
  .run summary { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; color: inherit; }
  .results { width: 100%; border-collapse: collapse; font-size: 12px; margin-top: 6px; }
  .results th, .results td { text-align: left; padding: 2px 6px; border-bottom: 1px solid var(--separator); }
  .results .pass { color: var(--green); }
  .results .fail { color: var(--red); }
  .results .unknown { color: var(--orange); }
  .link { color: var(--accent-text); }
  summary:focus-visible, .button:focus-visible, .row-head:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible { outline: var(--focus-ring, 2px solid var(--accent-text)); outline-offset: 2px; }
</style>
