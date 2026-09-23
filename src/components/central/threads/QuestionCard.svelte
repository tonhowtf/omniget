<script lang="ts">
  // Pending user question (AskUserQuestion) in the composer drawer: one
  // question at a time ("2/3"), digits 1–9 pick options, single-select
  // auto-advances after 200 ms, multi-select toggles. The composer text is
  // the custom answer when allowed (`customText`).
  import { t } from "$lib/i18n";
  import type { UserInputRow } from "$lib/central/threads/types";
  import Icon from "./Icon.svelte";

  let {
    input,
    customText = "",
    busy = false,
    onsubmit,
    ondismiss,
    oncollapse,
  }: {
    input: UserInputRow;
    customText?: string;
    busy?: boolean;
    onsubmit: (answers: Record<string, string | string[]>) => void;
    ondismiss?: () => void;
    oncollapse?: (collapsed: boolean) => void;
  } = $props();

  let step = $state(0);
  let picks = $state<Record<string, string[]>>({});
  let collapsed = $state(false);
  let advanceTimer: ReturnType<typeof setTimeout> | null = null;

  let q = $derived(input.questions[step] ?? null);
  let total = $derived(input.questions.length);
  let last = $derived(step >= total - 1);
  let dismissible = $derived(input.responseMode === "message");

  $effect(() => {
    input.requestId;
    step = 0;
    picks = {};
    collapsed = false;
  });

  function valueOf(o: { label: string; value?: string }) {
    return o.value ?? o.label;
  }

  function pick(i: number) {
    if (!q || busy) return;
    const o = q.options[i];
    if (!o) return;
    const v = valueOf(o);
    const cur = picks[q.id] ?? [];
    if (q.multiSelect) {
      picks = { ...picks, [q.id]: cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v] };
      return;
    }
    picks = { ...picks, [q.id]: [v] };
    if (advanceTimer) clearTimeout(advanceTimer);
    advanceTimer = setTimeout(() => (last ? submit() : (step += 1)), 200);
  }

  function answerOf(id: string, multi: boolean, allowCustom: boolean): string | string[] {
    const chosen = picks[id] ?? [];
    if (allowCustom && customText.trim() && id === q?.id) return multi ? [...chosen, customText.trim()] : customText.trim();
    return multi ? chosen : (chosen[0] ?? "");
  }

  function submit() {
    const answers: Record<string, string | string[]> = {};
    for (const qq of input.questions) answers[qq.id] = answerOf(qq.id, qq.multiSelect, qq.allowCustomAnswer);
    onsubmit(answers);
  }

  function isEditable(el: EventTarget | null): boolean {
    const n = el as HTMLElement | null;
    return !!n && (n.isContentEditable || n.tagName === "INPUT" || n.tagName === "TEXTAREA" || n.tagName === "SELECT");
  }

  function onKey(e: KeyboardEvent) {
    if (collapsed || e.metaKey || e.ctrlKey || e.altKey || isEditable(e.target)) return;
    if (/^[1-9]$/.test(e.key)) {
      e.preventDefault();
      pick(Number(e.key) - 1);
    }
  }

  function toggleCollapse() {
    collapsed = !collapsed;
    oncollapse?.(collapsed);
  }
</script>

<svelte:window onkeydown={onKey} />

{#if q}
  <section class="card" aria-label={$t("llm.central.threads.question.title")}>
    <div class="head">
      <button type="button" class="head-btn" aria-expanded={!collapsed} onclick={toggleCollapse}>
        <Icon name="chat-circle-dots" size={14} />
        <span class="header">{q.header}</span>
        {#if collapsed}<span class="q-short">{q.question}</span>{/if}
        {#if total > 1}<span class="count">{step + 1}/{total}</span>{/if}
        <span class="chev" class:open={!collapsed}><Icon name="caret-right" size={10} /></span>
      </button>
      {#if dismissible && ondismiss}
        <button type="button" class="x" aria-label={$t("llm.central.threads.question.dismiss")} onclick={ondismiss}><Icon name="x" size={12} /></button>
      {/if}
    </div>
    {#if !collapsed}
      <p class="question">{q.question}</p>
      {#if q.multiSelect}<p class="hint">{$t("llm.central.threads.question.multi")}</p>{/if}
      <div class="options" role={q.multiSelect ? "group" : "radiogroup"}>
        {#each q.options as o, i (i)}
          {@const on = (picks[q.id] ?? []).includes(valueOf(o))}
          <button type="button" class="opt" class:on role={q.multiSelect ? "checkbox" : "radio"} aria-checked={on} disabled={busy} onclick={() => pick(i)}>
            <span class="opt-text">
              <span class="opt-label">{o.label}</span>
              {#if o.description}<span class="opt-desc">{o.description}</span>{/if}
            </span>
            {#if on}<Icon name="check" size={13} />{:else if i < 9}<kbd>{i + 1}</kbd>{/if}
          </button>
        {/each}
      </div>
      {#if q.allowCustomAnswer}<p class="hint">{$t("llm.central.threads.question.custom_hint")}</p>{/if}
      <footer>
        {#if step > 0}<button type="button" class="button" onclick={() => (step -= 1)}>{$t("llm.central.threads.question.previous")}</button>{/if}
        {#if !last}
          <button type="button" class="button primary" onclick={() => (step += 1)}>{$t("llm.central.threads.question.next")}</button>
        {:else}
          <button type="button" class="button primary" disabled={busy} onclick={submit}>
            {busy ? $t("llm.central.threads.question.submitting") : $t("llm.central.threads.question.submit")}
          </button>
        {/if}
      </footer>
    {/if}
  </section>
{/if}

<style>
  .card { display: grid; gap: 8px; padding: 12px 14px; border-radius: 16px 16px 0 0; border: 1px solid color-mix(in srgb, var(--purple, #8b5cf6) 40%, var(--separator)); border-bottom: 0; background: color-mix(in srgb, var(--purple, #8b5cf6) 6%, var(--surface, #fff)); }
  .head { display: flex; align-items: center; gap: 4px; }
  .head-btn { flex: 1; min-width: 0; display: flex; align-items: center; gap: 7px; border: 0; background: transparent; padding: 0; color: var(--purple, #8b5cf6); cursor: pointer; font-size: 12.5px; text-align: left; }
  .header { font-weight: 650; }
  .q-short { color: var(--text-muted); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { margin-left: auto; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .chev { display: inline-flex; transition: transform 150ms; color: var(--text-muted); }
  .chev.open { transform: rotate(90deg); }
  .x { border: 0; background: transparent; color: var(--text-muted); padding: 3px; border-radius: 5px; cursor: pointer; display: inline-flex; }
  .question { margin: 0; font-size: 14px; color: var(--text); font-weight: 500; }
  .hint { margin: 0; font-size: 11.5px; color: var(--text-muted); }
  .options { display: grid; gap: 4px; }
  .opt { display: flex; align-items: center; gap: 10px; justify-content: space-between; text-align: left; border: 1px solid var(--separator); background: var(--surface, #fff); color: var(--text); padding: 8px 10px; border-radius: 10px; cursor: pointer; }
  .opt:hover:not(:disabled) { border-color: var(--purple, #8b5cf6); }
  .opt.on { border-color: var(--purple, #8b5cf6); background: color-mix(in srgb, var(--purple, #8b5cf6) 10%, var(--surface, #fff)); }
  .opt-text { display: grid; gap: 2px; min-width: 0; }
  .opt-label { font-size: 13px; font-weight: 500; }
  .opt-desc { font-size: 12px; color: var(--text-muted); }
  kbd { font-family: var(--font-mono); font-size: 10.5px; padding: 1px 6px; border-radius: 5px; background: var(--fill-2); color: var(--text-muted); }
  footer { display: flex; gap: 6px; justify-content: flex-end; }
</style>
