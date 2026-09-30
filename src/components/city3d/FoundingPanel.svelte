<script lang="ts">
  /**
   * Founding the 3D city (M3): pick one of the AI connections, write a brief
   * and follow the loop live (the model writes the charter and the first
   * program, the generator checks it, the violations go back) — or found the
   * example city without an LLM. With a city already founded, the panel says
   * so and keeps re-founding behind a confirmation.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import ModelPicker from "$components/llm/ModelPicker.svelte";
  import type { ModelRef } from "$lib/llm/types";
  import * as city from "$lib/city3d/api";
  import type { FoundingEvent, ModelChoice, CityStatus } from "$lib/city3d/api";
  import type { Violation } from "$lib/city3d/types";

  let { onfounded }: { onfounded?: () => void } = $props();

  let status = $state<CityStatus | null>(null);
  let choices = $state<ModelChoice[]>([]);
  let chosen = $state<string>("");
  let custom = $state<ModelRef | null>(null);
  let useCustom = $state(false);
  let brief = $state("");
  let refound = $state(false);

  let job = $state<string | null>(null);
  let running = $state(false);
  let lines = $state<FoundingEvent[]>([]);
  let violations = $state<Violation[]>([]);
  let error = $state<string | null>(null);
  let done = $state(false);

  const founded = $derived(!!status?.city);
  const last = $derived(lines.length ? lines[lines.length - 1] : null);
  const errors = $derived(violations.filter((v) => v.severity === "error"));
  const warnings = $derived(violations.filter((v) => v.severity !== "error"));

  function selected(): { provider: string | null; model: string | null } {
    if (useCustom && custom?.provider) return { provider: custom.provider, model: custom.model || null };
    const pick = choices.find((c) => `${c.provider}/${c.model}` === chosen);
    return pick ? { provider: pick.provider, model: pick.model } : { provider: null, model: null };
  }

  async function load(): Promise<void> {
    try {
      status = await city.status();
    } catch {
      status = null;
    }
    try {
      choices = await city.providers();
      if (!chosen && choices.length) chosen = `${choices[0].provider}/${choices[0].model}`;
      if (!choices.length) useCustom = true;
    } catch {
      choices = [];
      useCustom = true;
    }
  }

  function message(e: unknown): string {
    const text = String((e as { message?: string })?.message ?? e);
    switch (city.errorCode(text)) {
      case "ERR_CITY3D_NO_KEY":
        return $t("city3d.found.err_no_key") as string;
      case "ERR_CITY3D_NO_PROVIDER":
        return $t("city3d.found.err_no_provider") as string;
      case "ERR_CITY3D_NO_MODEL":
        return $t("city3d.found.err_no_model") as string;
      default:
        return text;
    }
  }

  async function start(): Promise<void> {
    error = null;
    done = false;
    lines = [];
    violations = [];
    running = true;
    try {
      const { provider, model } = selected();
      const r = await city.foundStart({ brief, provider, model });
      job = r.job;
    } catch (e) {
      error = message(e);
      running = false;
    }
  }

  async function cancel(): Promise<void> {
    if (job) await city.foundCancel(job).catch(() => {});
  }

  async function example(): Promise<void> {
    error = null;
    running = true;
    lines = [];
    violations = [];
    try {
      const r = await city.foundExample();
      violations = r.violations;
      if (r.founded) {
        done = true;
        await load();
        onfounded?.();
      } else {
        error = $t("city3d.found.example_failed") as string;
      }
    } catch (e) {
      error = message(e);
    } finally {
      running = false;
    }
  }

  onMount(() => {
    void load();
    let stop: (() => void) | null = null;
    void city
      .onFounding((ev) => {
        if (!job || ev.job !== job) return;
        lines = [...lines.slice(-40), ev];
        if (ev.violations?.length || ev.stage === "retry") violations = ev.violations ?? [];
        if (ev.done) {
          running = false;
          if (ev.error) {
            error = ev.error;
          } else {
            done = true;
            refound = false;
            void load();
            onfounded?.();
          }
        }
      })
      .then((un) => (stop = un))
      .catch(() => {});
    return () => stop?.();
  });
</script>

<div class="found">
  {#if founded && !refound}
    <section class="state">
      <p class="title">{$t("city3d.found.founded_title", { name: status?.city?.name ?? status?.city?.city_id ?? "" })}</p>
      {#if status?.city?.charter?.story}<p class="story">{status.city.charter.story}</p>{/if}
      <p class="note">
        {$t("city3d.found.founded_meta", {
          regions: status?.city?.regions?.length ?? 1,
          policy: status?.city?.publish_policy ?? "auto",
        })}
      </p>
      {#if status?.mode === "local"}
        <button type="button" class="button" onclick={() => (refound = true)}>{$t("city3d.found.refound")}</button>
      {/if}
    </section>
  {:else}
    <section class="state">
      <p class="title">{$t("city3d.found.unfounded_title")}</p>
      <p class="note">{$t("city3d.found.unfounded_body")}</p>
      {#if founded}<p class="warn" role="alert">{$t("city3d.found.refound_warning")}</p>{/if}
    </section>

    <fieldset class="agent" disabled={running}>
      <legend class="field-label">{$t("city3d.found.agent")}</legend>
      {#if choices.length}
        <div class="choices" role="radiogroup" aria-label={$t("city3d.found.agent") as string}>
          {#each choices as c (c.provider + "/" + c.model)}
            {@const key = `${c.provider}/${c.model}`}
            <label class="choice" class:active={!useCustom && chosen === key}>
              <input type="radio" name="city3d-agent" value={key} checked={!useCustom && chosen === key} onchange={() => { chosen = key; useCustom = false; }} />
              <span class="choice-model">{c.model}</span>
              <span class="choice-provider">{c.provider}{c.local ? ` · ${$t("city3d.found.local")}` : ""}</span>
            </label>
          {/each}
          <label class="choice" class:active={useCustom}>
            <input type="radio" name="city3d-agent" checked={useCustom} onchange={() => (useCustom = true)} />
            <span class="choice-model">{$t("city3d.found.other_model")}</span>
          </label>
        </div>
      {:else}
        <p class="note">{$t("city3d.found.no_connections")}</p>
      {/if}
      {#if useCustom}
        <ModelPicker value={custom} busy={running} onchange={(ref) => (custom = ref)} />
      {/if}
    </fieldset>

    <label class="field">
      <span class="field-label">{$t("city3d.found.brief")}</span>
      <textarea class="input brief" rows="4" bind:value={brief} disabled={running} placeholder={$t("city3d.found.brief_placeholder") as string}></textarea>
    </label>

    <div class="actions">
      {#if running && job}
        <button type="button" class="button" onclick={cancel}>{$t("city3d.found.cancel")}</button>
      {:else}
        <button type="button" class="button primary" disabled={running} onclick={start}>{$t("city3d.found.start")}</button>
        <button type="button" class="button" disabled={running} onclick={example}>{$t("city3d.found.example")}</button>
        {#if founded}<button type="button" class="button" disabled={running} onclick={() => (refound = false)}>{$t("city3d.found.keep")}</button>{/if}
      {/if}
    </div>
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if lines.length}
    <section class="progress" aria-live="polite">
      <p class="field-label">
        {$t("city3d.found.progress")}
        {#if last}· {$t("city3d.found.attempt", { count: last.attempt })} · {$t("city3d.found.tokens", { count: last.tokens })}{/if}
      </p>
      <ol class="lines">
        {#each lines as l, i (i)}
          <li class:done={l.done && !l.error} class:failed={!!l.error}>
            <span class="stage">{$t(`city3d.found.stage_${l.stage}`)}</span>
            <span>{l.message}</span>
          </li>
        {/each}
      </ol>
      {#if running}<p class="note">{$t("city3d.found.running")}</p>{/if}
      {#if done}<p class="ok">{$t("city3d.found.done")}</p>{/if}
    </section>
  {/if}

  {#if violations.length}
    <section class="violations">
      <p class="field-label">{$t("city3d.found.violations", { errors: errors.length, warnings: warnings.length })}</p>
      <ul>
        {#each violations.slice(0, 40) as v, i (i)}
          <li class:warning={v.severity !== "error"}>
            <code>{v.code}</code> <code>{v.id}</code>{#if v.measure} <span class="measure">{v.measure}</span>{/if}
            <span class="msg">{v.message}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .found {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 0.75rem);
    font-size: var(--text-sm, 0.875rem);
  }
  .state .title {
    font-weight: 600;
    font-size: var(--text-md, 1rem);
    margin: 0 0 var(--space-1, 0.25rem);
  }
  .story {
    margin: 0 0 var(--space-2, 0.5rem);
    color: var(--text-secondary, inherit);
  }
  .note {
    margin: 0;
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
    font-size: var(--text-caption, 0.8rem);
  }
  .warn,
  .error {
    margin: 0;
    color: var(--danger, #d70015);
  }
  .ok {
    margin: 0;
    color: var(--success, #248a3d);
  }
  fieldset.agent {
    border: 0;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
    min-width: 0;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
  }
  .choice {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: var(--space-2, 0.5rem);
    align-items: center;
    padding: var(--space-2, 0.5rem);
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
    cursor: pointer;
  }
  .choice.active {
    border-color: var(--accent, #0a84ff);
  }
  .choice input {
    grid-row: span 2;
  }
  .choice-model {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .choice-provider {
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
    font-size: var(--text-caption, 0.8rem);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
  }
  .brief {
    resize: vertical;
    min-height: 5rem;
    font: inherit;
  }
  .actions {
    display: flex;
    gap: var(--space-2, 0.5rem);
    flex-wrap: wrap;
  }
  .lines {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 12rem;
    overflow: auto;
  }
  .lines li {
    display: flex;
    gap: var(--space-2, 0.5rem);
  }
  .lines .stage {
    flex: none;
    min-width: 6.5rem;
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
  }
  .lines li.done .stage {
    color: var(--success, #248a3d);
  }
  .lines li.failed .stage {
    color: var(--danger, #d70015);
  }
  .violations ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
    max-height: 14rem;
    overflow: auto;
  }
  .violations li {
    padding-left: var(--space-2, 0.5rem);
    border-left: 2px solid var(--danger, #d70015);
  }
  .violations li.warning {
    border-left-color: var(--warning, #b25000);
  }
  .violations code {
    font-size: var(--text-caption, 0.8rem);
  }
  .measure {
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
  }
  .msg {
    display: block;
  }
</style>
