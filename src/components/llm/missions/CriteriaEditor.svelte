<script lang="ts">
  /**
   * The completion criteria of a mission, as the person writes them: what
   * must be true for the mission to count as done. Objective checks (a
   * command, a file, a tool result) decide on their own; a rubric or a human
   * check waits for the person. A command imported or proposed by a model is
   * shown but never runs.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getAgents } from "$lib/stores/llm-store.svelte";
  import {
    CRITERION_KINDS, criterionExecutable, lines, newCriterion,
    type Criterion, type CriterionKind,
  } from "$lib/stores/assist-missions-store.svelte";

  let { criteria = $bindable([]), disabled = false }: { criteria?: Criterion[]; disabled?: boolean } = $props();

  let addKind = $state<CriterionKind>("command");
  let journeys = $state<{ id: string; title: string; bot_id: string }[]>([]);
  let journeysFailed = $state(false);
  let agents = $derived(getAgents());

  onMount(async () => {
    try {
      journeys = await invoke<{ id: string; title: string; bot_id: string }[]>("assist_reading_list_all");
    } catch {
      journeysFailed = true;
    }
  });

  function str(c: Criterion, key: string): string {
    const v = c.spec?.[key];
    return v == null ? "" : String(v);
  }
  function list(c: Criterion, key: string): string {
    const v = c.spec?.[key];
    return Array.isArray(v) ? v.join("\n") : "";
  }
  function num(c: Criterion, key: string): string {
    const v = c.spec?.[key];
    return typeof v === "number" ? String(v) : "";
  }
  function setSpec(i: number, key: string, value: unknown) {
    const next = { ...criteria[i].spec };
    if (value === "" || value == null || (Array.isArray(value) && value.length === 0)) delete next[key];
    else next[key] = value;
    criteria[i] = { ...criteria[i], spec: next };
  }
  function setField<K extends keyof Criterion>(i: number, key: K, value: Criterion[K]) {
    criteria[i] = { ...criteria[i], [key]: value };
  }
  function setKind(i: number, kind: CriterionKind) {
    const fresh = newCriterion(kind);
    criteria[i] = { ...criteria[i], kind, spec: fresh.spec, acceptance: fresh.acceptance };
  }
  function setNum(i: number, key: string, raw: string) {
    const n = Number(raw);
    setSpec(i, key, raw.trim() && Number.isFinite(n) && n >= 0 ? Math.round(n) : null);
  }
  function add() {
    criteria = [...criteria, newCriterion(addKind)];
  }
  function remove(i: number) {
    criteria = criteria.filter((_, j) => j !== i);
  }
</script>

<div class="criteria">
  <p class="hint">{$t("mission.criteria.lede")}</p>
  {#if criteria.length === 0}
    <p class="hint empty">{$t("mission.criteria.empty")}</p>
  {/if}
  {#each criteria as c, i (c.id)}
    <fieldset class="crit" {disabled}>
      <legend class="sr">{$t("mission.criteria.item", { count: i + 1 })}</legend>
      <div class="crit-head">
        <label class="field grow">
          <span class="field-label">{$t("mission.criteria.title")}</span>
          <input class="input" type="text" value={c.title} placeholder={$t("mission.criteria.title_placeholder")}
            oninput={(e) => setField(i, "title", e.currentTarget.value)} />
        </label>
        <label class="field">
          <span class="field-label">{$t("mission.criteria.kind")}</span>
          <select class="input" value={c.kind} onchange={(e) => setKind(i, e.currentTarget.value as CriterionKind)}>
            {#each CRITERION_KINDS as k (k)}<option value={k}>{$t(`mission.kind.${k}`)}</option>{/each}
          </select>
        </label>
        <label class="field">
          <span class="field-label">{$t("mission.criteria.severity")}</span>
          <select class="input" value={c.severity} onchange={(e) => setField(i, "severity", e.currentTarget.value as Criterion["severity"])}>
            <option value="required">{$t("mission.severity.required")}</option>
            <option value="advisory">{$t("mission.severity.advisory")}</option>
          </select>
        </label>
        <button type="button" class="button remove" aria-label={$t("mission.criteria.remove")} title={$t("mission.criteria.remove")} onclick={() => remove(i)}>×</button>
      </div>
      <p class="hint">{$t(`mission.kind_hint.${c.kind}`)}</p>
      <div class="tags">
        {#if c.origin !== "user"}<span class="tag">{$t(`mission.origin.${c.origin}`)}</span>{/if}
        {#if !criterionExecutable(c)}<span class="tag warn">{$t("mission.criteria.not_executable")}</span>{/if}
        {#if c.kind === "rubric" || c.kind === "human"}<span class="tag">{$t("mission.criteria.waits_for_you")}</span>{/if}
      </div>

      {#if c.kind === "command"}
        <label class="field">
          <span class="field-label">{$t("mission.criteria.command")}</span>
          <input class="input mono" type="text" value={str(c, "command")} placeholder="pnpm test" spellcheck="false"
            oninput={(e) => setSpec(i, "command", e.currentTarget.value)} />
        </label>
        <details class="more">
          <summary>{$t("mission.criteria.more")}</summary>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.timeout")}</span>
            <input class="input narrow" type="number" min="0" value={num(c, "timeout_ms")} onchange={(e) => setNum(i, "timeout_ms", e.currentTarget.value)} />
          </label>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.artifacts")}</span>
            <textarea class="input mono" rows="2" value={list(c, "artifacts")} onchange={(e) => setSpec(i, "artifacts", lines(e.currentTarget.value))}></textarea>
          </label>
        </details>
      {:else if c.kind === "artifact"}
        <label class="field">
          <span class="field-label">{$t("mission.criteria.path")}</span>
          <input class="input mono" type="text" value={str(c, "path")} placeholder="docs/report.md" spellcheck="false"
            oninput={(e) => setSpec(i, "path", e.currentTarget.value)} />
        </label>
        <label class="field">
          <span class="field-label">{$t("mission.criteria.contains")}</span>
          <textarea class="input" rows="2" value={list(c, "contains")} onchange={(e) => setSpec(i, "contains", lines(e.currentTarget.value))}></textarea>
        </label>
        <details class="more">
          <summary>{$t("mission.criteria.more")}</summary>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.not_contains")}</span>
            <textarea class="input" rows="2" value={list(c, "not_contains")} onchange={(e) => setSpec(i, "not_contains", lines(e.currentTarget.value))}></textarea>
          </label>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.json_keys")}</span>
            <textarea class="input mono" rows="2" value={list(c, "json_keys")} onchange={(e) => setSpec(i, "json_keys", lines(e.currentTarget.value))}></textarea>
          </label>
          <label class="check">
            <input type="checkbox" checked={c.spec?.must_exist !== false} onchange={(e) => setSpec(i, "must_exist", e.currentTarget.checked)} />
            <span>{$t("mission.criteria.must_exist")}</span>
          </label>
        </details>
      {:else if c.kind === "tool_result"}
        <label class="field">
          <span class="field-label">{$t("mission.criteria.journey")}</span>
          <select class="input" value={str(c, "journey_id")} onchange={(e) => { setSpec(i, "check", "reading_round"); setSpec(i, "journey_id", e.currentTarget.value); }}>
            <option value="">—</option>
            {#each journeys as j (j.id)}<option value={j.id}>{j.title} · {agents.find((a) => a.id === j.bot_id)?.name ?? j.bot_id}</option>{/each}
          </select>
        </label>
        {#if journeysFailed || journeys.length === 0}<p class="hint">{$t("mission.criteria.no_journeys")}</p>{/if}
        <div class="row-fields">
          <label class="field">
            <span class="field-label">{$t("mission.criteria.min_items")}</span>
            <input class="input narrow" type="number" min="0" value={num(c, "min_items")} onchange={(e) => setNum(i, "min_items", e.currentTarget.value)} />
          </label>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.max_items")}</span>
            <input class="input narrow" type="number" min="0" value={num(c, "max_items")} onchange={(e) => setNum(i, "max_items", e.currentTarget.value)} />
          </label>
        </div>
        <details class="more">
          <summary>{$t("mission.criteria.more")}</summary>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.first_round_roles")}</span>
            <textarea class="input" rows="2" value={list(c, "first_round_roles")} onchange={(e) => setSpec(i, "first_round_roles", lines(e.currentTarget.value))}></textarea>
          </label>
        </details>
      {:else if c.kind === "rubric"}
        <label class="field">
          <span class="field-label">{$t("mission.criteria.rubric")}</span>
          <textarea class="input" rows="3" value={list(c, "rubric")} placeholder={$t("mission.criteria.rubric_placeholder")}
            onchange={(e) => setSpec(i, "rubric", lines(e.currentTarget.value))}></textarea>
        </label>
        <div class="row-fields">
          <label class="field">
            <span class="field-label">{$t("mission.criteria.reviewer")}</span>
            <select class="input" value={str(c, "reviewer")} onchange={(e) => setSpec(i, "reviewer", e.currentTarget.value)}>
              <option value="">{$t("mission.criteria.reviewer_none")}</option>
              {#each agents as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{$t("mission.criteria.acceptance")}</span>
            <select class="input" value={c.acceptance === "single_review" ? "single_review" : "human"}
              onchange={(e) => setField(i, "acceptance", e.currentTarget.value as Criterion["acceptance"])}>
              <option value="human">{$t("mission.acceptance.human")}</option>
              <option value="single_review">{$t("mission.acceptance.single_review")}</option>
            </select>
          </label>
        </div>
      {:else}
        <label class="field">
          <span class="field-label">{$t("mission.criteria.prompt")}</span>
          <textarea class="input" rows="2" value={str(c, "prompt")} oninput={(e) => setSpec(i, "prompt", e.currentTarget.value)}></textarea>
        </label>
      {/if}
    </fieldset>
  {/each}

  <div class="actions">
    <select class="input narrow" bind:value={addKind} aria-label={$t("mission.criteria.kind")} {disabled}>
      {#each CRITERION_KINDS as k (k)}<option value={k}>{$t(`mission.kind.${k}`)}</option>{/each}
    </select>
    <button type="button" class="button" {disabled} onclick={add}>{$t("mission.criteria.add")}</button>
  </div>
</div>

<style>
  .criteria { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .crit { display: flex; flex-direction: column; gap: var(--space-2); margin: 0; padding: var(--space-3); border: var(--hairline) solid var(--separator); border-radius: var(--radius-md); min-width: 0; }
  .crit-head, .row-fields { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: flex-end; }
  .field { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .grow { flex: 1 1 220px; }
  .narrow { max-width: 180px; }
  .mono { font-family: var(--font-mono); }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  textarea.mono { font-family: var(--font-mono); }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .empty { font-style: italic; }
  .tags { display: flex; flex-wrap: wrap; gap: 6px; }
  .tags:empty { display: none; }
  .tag { padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .tag.warn { color: var(--orange); background: color-mix(in srgb, var(--orange) 14%, transparent); }
  .remove { min-width: 32px; }
  .check { display: inline-flex; gap: 6px; align-items: center; font-size: var(--text-sm); }
  .more summary { cursor: pointer; font-size: var(--text-sm); color: var(--accent-text); }
  .more[open] { display: flex; flex-direction: column; gap: var(--space-2); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  summary:focus-visible, .button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
