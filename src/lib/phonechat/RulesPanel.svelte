<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { WaLimits, WaRule, WaRuleRun, WaRulesState, WaRuleTest } from "./api";
  import { wa } from "./session.svelte";
  import WaIcon from "./WaIcon.svelte";
  import WaModal from "./WaModal.svelte";
  import CopyButton from "./CopyButton.svelte";

  let { onclose }: { onclose: () => void } = $props();

  type Form = {
    id: number;
    name: string;
    trigger: "keyword" | "first_message" | "outside_hours" | "any";
    open: string;
    close: string;
    needsTag: string;
    question: string;
    askField: string;
    aiDraft: boolean;
    mode: "contains" | "exact" | "starts_with";
    words: string;
    action: "reply" | "react" | "ai";
    text: string;
    emoji: string;
    instructions: string;
    fallback: string;
    cooldown: number;
    chats: string[];
    enabled: boolean;
    tag: string;
    followUps: { after_minutes: number; text: string }[];
  };

  let view = $state<WaRulesState | null>(null);
  let runs = $state<WaRuleRun[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let form = $state<Form | null>(null);
  let probe = $state("");
  let probed = $state<WaRuleTest | null>(null);
  let loads = 0;

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? ($t(`phonechat.errors.${k}`) as string) : raw;
  }

  async function load() {
    const mine = ++loads;
    try {
      const [s, r] = await Promise.all([api.rules(), api.ruleRuns(20)]);
      if (mine !== loads) return;
      view = s;
      runs = r;
    } catch (e) {
      if (mine === loads) error = api.errText(e);
    }
  }

  $effect(() => {
    void wa.rulesVersion;
    untrack(() => void load());
  });

  async function act(work: () => Promise<unknown>) {
    busy = true;
    error = null;
    try {
      await work();
      await load();
    } catch (e) {
      error = api.errText(e);
    } finally {
      busy = false;
    }
  }

  function setLimits(patch: Partial<WaLimits>) {
    if (!view) return;
    const next = { ...view.limits, ...patch };
    view = { ...view, limits: next };
    void act(() => api.rulesLimits(next));
  }

  function numberOf(e: Event, fallback: number): number {
    const n = Number((e.target as HTMLInputElement).value);
    return Number.isFinite(n) && n > 0 ? Math.round(n) : fallback;
  }

  function blank(): Form {
    return { id: 0, name: "", trigger: "keyword", mode: "contains", words: "", action: "reply", text: "", emoji: "👍", instructions: "", fallback: "", cooldown: 60, chats: [], enabled: true, tag: "", followUps: [], open: "09:00", close: "18:00", needsTag: "", question: "", askField: "", aiDraft: false };
  }

  function edit(rule: WaRule) {
    const f = blank();
    f.id = rule.id;
    f.name = rule.name;
    f.enabled = rule.enabled;
    f.cooldown = rule.cooldown_minutes;
    f.chats = rule.chats;
    f.tag = rule.tag ?? "";
    f.followUps = rule.follow_ups.map((x) => ({ ...x }));
    f.trigger = rule.trigger.kind;
    f.needsTag = rule.needs_tag ?? "";
    f.question = rule.ask?.question ?? "";
    f.askField = rule.ask?.field ?? "";
    if (rule.trigger.kind === "outside_hours") {
      f.open = clock(rule.trigger.open_minute);
      f.close = clock(rule.trigger.close_minute);
    }
    if (rule.trigger.kind === "keyword") {
      f.mode = rule.trigger.mode;
      f.words = rule.trigger.words.join(", ");
    }
    f.action = rule.action.kind;
    if (rule.action.kind === "reply") f.text = rule.action.text;
    if (rule.action.kind === "react") f.emoji = rule.action.emoji;
    if (rule.action.kind === "ai") {
      f.instructions = rule.action.instructions;
      f.fallback = rule.action.fallback ?? "";
      f.aiDraft = rule.action.draft;
    }
    form = f;
  }

  function clock(minute: number): string {
    return `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
  }

  function minutes(time: string): number {
    const [h, m] = time.split(":").map(Number);
    return (h || 0) * 60 + (m || 0);
  }

  function toRule(f: Form): WaRule {
    return {
      id: f.id,
      name: f.name,
      enabled: f.enabled,
      chats: f.chats,
      cooldown_minutes: Math.max(0, Math.round(f.cooldown) || 0),
      tag: f.tag.trim() || null,
      follow_ups: f.followUps.filter((x) => x.text.trim()).map((x) => ({ after_minutes: Math.max(1, Math.round(x.after_minutes) || 1), text: x.text })),
      trigger:
        f.trigger === "keyword"
          ? { kind: "keyword", mode: f.mode, words: f.words.split(/[,\n]/).map((w) => w.trim()).filter(Boolean) }
          : f.trigger === "outside_hours"
            ? { kind: "outside_hours", open_minute: minutes(f.open), close_minute: minutes(f.close) }
            : { kind: f.trigger },
      action:
        f.action === "reply"
          ? { kind: "reply", text: f.text }
          : f.action === "react"
            ? { kind: "react", emoji: f.emoji }
            : { kind: "ai", instructions: f.instructions, fallback: f.fallback.trim() || null, draft: f.aiDraft },
      needs_tag: f.needsTag.trim() || null,
      ask: f.question.trim() ? { question: f.question, field: f.askField } : null,
    };
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!form) return;
    const rule = toRule(form);
    await act(async () => {
      await api.ruleSave(rule);
      form = null;
    });
  }

  async function test(event: SubmitEvent) {
    event.preventDefault();
    if (!probe.trim()) return;
    error = null;
    try {
      probed = await api.ruleTest(probe);
    } catch (e) {
      error = api.errText(e);
    }
  }

  function summary(rule: WaRule): string {
    const when =
      rule.trigger.kind === "keyword"
        ? `${$t(`phonechat.rules.mode_${rule.trigger.mode}`)}: ${rule.trigger.words.join(", ")}`
        : rule.trigger.kind === "outside_hours"
          ? `${$t("phonechat.rules.trigger_hours")} (${clock(rule.trigger.open_minute)}–${clock(rule.trigger.close_minute)})`
          : ($t(`phonechat.rules.trigger_${rule.trigger.kind === "first_message" ? "first" : "any"}`) as string);
    const then =
      rule.action.kind === "reply" ? rule.action.text.split("\n")[0] : rule.action.kind === "react" ? rule.action.emoji : ($t("phonechat.rules.action_ai") as string);
    return `${when} → ${then}`;
  }

  function entryLink(rule: WaRule): string | null {
    const phone = wa.status?.account?.jid?.split("@")[0]?.split(":")[0] ?? "";
    if (rule.trigger.kind !== "keyword" || !/^\d{8,15}$/.test(phone)) return null;
    return `https://wa.me/${phone}?text=${encodeURIComponent(rule.trigger.words[0] ?? "")}`;
  }

  let steps = $derived(
    (view?.rules ?? [])
      .filter((r) => r.tag)
      .map((r) => ({ from: r, tag: r.tag ?? "", next: (view?.rules ?? []).filter((n) => n.needs_tag === r.tag) }))
      .filter((s) => s.next.length > 0),
  );

  function statOf(id: number) {
    const s = view?.stats.find((x) => x.rule_id === id);
    return s && s.sent + s.skipped + s.failed > 0 ? s : null;
  }

  function ruleName(id: number): string {
    return view?.rules.find((r) => r.id === id)?.name ?? `#${id}`;
  }

  function detail(run: WaRuleRun): string {
    if (!run.detail) return "";
    const code = run.detail.split(":")[0];
    const known = ["cooldown", "human_active", "hour_cap", "day_cap", "ai_silent", "follow_up", "captured", "draft"].includes(code) ? code : code.startsWith("ai_") ? "ai_failed" : null;
    return known ? ($t(`phonechat.rules.reason.${known}`) as string) : errorLabel(run.detail);
  }

  function when(ts: number): string {
    return new Date(ts * 1000).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  }
</script>

<WaModal title={$t("phonechat.rules.title")} subtitle={$t("phonechat.rules.subtitle")} width={640} {onclose}>
  {#if !view}
    <div class="center">{#if error}<span class="err">{errorLabel(error)}</span>{:else}<span class="spinner"></span>{/if}</div>
  {:else if form}
    <form class="editor" onsubmit={save}>
      <label class="field">
        <span>{$t("phonechat.rules.name")}</span>
        <input class="input" bind:value={form.name} maxlength="80" required />
      </label>
      <div class="field">
        <span id="rule-trigger">{$t("phonechat.rules.trigger")}</span>
        <select class="select" aria-labelledby="rule-trigger" bind:value={form.trigger}>
          <option value="keyword">{$t("phonechat.rules.trigger_keyword")}</option>
          <option value="first_message">{$t("phonechat.rules.trigger_first")}</option>
          <option value="outside_hours">{$t("phonechat.rules.trigger_hours")}</option>
          <option value="any">{$t("phonechat.rules.trigger_any")}</option>
        </select>
      </div>
      {#if form.trigger === "keyword"}
        <div class="pair">
          <select class="select" aria-label={$t("phonechat.rules.trigger")} bind:value={form.mode}>
            <option value="contains">{$t("phonechat.rules.mode_contains")}</option>
            <option value="exact">{$t("phonechat.rules.mode_exact")}</option>
            <option value="starts_with">{$t("phonechat.rules.mode_starts_with")}</option>
          </select>
          <input class="input grow" bind:value={form.words} placeholder={$t("phonechat.rules.words")} aria-label={$t("phonechat.rules.words")} required />
        </div>
      {/if}
      {#if form.trigger === "outside_hours"}
        <div class="pair">
          <label class="field"><span>{$t("phonechat.rules.hours_open")}</span><input class="input" type="time" bind:value={form.open} required /></label>
          <label class="field"><span>{$t("phonechat.rules.hours_close")}</span><input class="input" type="time" bind:value={form.close} required /></label>
        </div>
      {/if}
      <div class="field">
        <span id="rule-action">{$t("phonechat.rules.action")}</span>
        <select class="select" aria-labelledby="rule-action" bind:value={form.action}>
          <option value="reply">{$t("phonechat.rules.action_reply")}</option>
          <option value="ai">{$t("phonechat.rules.action_ai")}</option>
          <option value="react">{$t("phonechat.rules.action_react")}</option>
        </select>
      </div>
      {#if form.action === "reply"}
        <label class="field">
          <span>{$t("phonechat.rules.text")}</span>
          <textarea class="input area" rows="4" bind:value={form.text} maxlength="4000" required></textarea>
          <small><code>{"{{first_name}}"}</code> {$t("phonechat.rules.text_hint")}</small>
        </label>
      {:else if form.action === "ai"}
        <label class="field">
          <span>{$t("phonechat.rules.instructions")}</span>
          <textarea class="input area" rows="5" bind:value={form.instructions} maxlength="4000" required></textarea>
          <small>{$t("phonechat.rules.ai_hint")}</small>
        </label>
        <label class="field">
          <span>{$t("phonechat.rules.fallback")}</span>
          <input class="input" bind:value={form.fallback} maxlength="4000" />
        </label>
        <label class="check"><input type="checkbox" bind:checked={form.aiDraft} /> <span>{$t("phonechat.rules.ai_draft")}</span></label>
      {:else}
        <label class="field narrow">
          <span>{$t("phonechat.rules.emoji")}</span>
          <input class="input" bind:value={form.emoji} maxlength="8" required />
        </label>
      {/if}
      <label class="field narrow">
        <span>{$t("phonechat.rules.cooldown")}</span>
        <input class="input" type="number" min="0" max="10080" bind:value={form.cooldown} />
      </label>
      {#if form.action !== "react"}
        <fieldset class="follow">
          <legend>{$t("phonechat.rules.follow_ups")}</legend>
          {#each form.followUps as step, i (i)}
            <div class="pair">
              <input class="input mins" type="number" min="1" max="10080" bind:value={step.after_minutes} aria-label={$t("phonechat.rules.follow_after")} title={$t("phonechat.rules.follow_after")} />
              <input class="input grow" bind:value={step.text} maxlength="4000" placeholder={$t("phonechat.rules.follow_text")} aria-label={$t("phonechat.rules.follow_text")} />
              <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.rules.follow_remove")} title={$t("phonechat.rules.follow_remove")} onclick={() => form && (form.followUps = form.followUps.filter((_, n) => n !== i))}><WaIcon name="x" /></button>
            </div>
          {/each}
          {#if form.followUps.length < 5}
            <button class="btn btn-ghost btn-sm add" type="button" onclick={() => form && (form.followUps = [...form.followUps, { after_minutes: 60, text: "" }])}><WaIcon name="plus" /> {$t("phonechat.rules.follow_add")}</button>
          {/if}
        </fieldset>
      {/if}
      {#if form.action !== "react"}
        <div class="pair ask">
          <label class="field grow"><span>{$t("phonechat.rules.ask")}</span><input class="input" bind:value={form.question} maxlength="4000" /></label>
          <label class="field"><span>{$t("phonechat.rules.ask_field")}</span><input class="input" bind:value={form.askField} maxlength="30" required={!!form.question.trim()} /></label>
        </div>
      {/if}
      <label class="field narrow">
        <span>{$t("phonechat.rules.tag")}</span>
        <input class="input" bind:value={form.tag} maxlength="30" />
      </label>
      <label class="field narrow">
        <span>{$t("phonechat.rules.needs_tag")}</span>
        <input class="input" bind:value={form.needsTag} maxlength="30" />
      </label>
      {#if error}<p class="err" role="alert">{errorLabel(error)}</p>{/if}
      <div class="row end">
        <button class="btn btn-ghost" type="button" onclick={() => ((form = null), (error = null))}>{$t("phonechat.cancel")}</button>
        <button class="btn btn-primary" type="submit" disabled={busy}>{#if busy}<span class="spinner"></span>{/if}{$t("phonechat.rules.save")}</button>
      </div>
    </form>
  {:else}
    <div class="master">
      <div class="master-copy">
        <span class="master-title" id="rules-switch">{$t("phonechat.rules.switch")}</span>
        <span class="muted">{$t("phonechat.rules.sent", { hour: view.sent_hour, cap: view.limits.per_hour, day: view.sent_day })}</span>
      </div>
      <button type="button" class="toggle" class:on={view.limits.enabled} role="switch" aria-checked={view.limits.enabled} aria-labelledby="rules-switch" disabled={busy}
        onclick={() => setLimits({ enabled: !view?.limits.enabled })}><span class="toggle-knob"></span></button>
    </div>
    <p class="risk"><WaIcon name="warn" size={14} /> <span>{$t("phonechat.rules.risk")}</span></p>

    {#if view.rules.length === 0}
      <div class="empty">
        <div class="empty-view-title">{$t("phonechat.rules.empty_title")}</div>
        <div class="empty-view-hint">{$t("phonechat.rules.empty_hint")}</div>
      </div>
    {:else}
      <ul class="rules">
        {#each view.rules as rule (rule.id)}
          <li class:off={!rule.enabled}>
            <button type="button" class="toggle" class:on={rule.enabled} role="switch" aria-checked={rule.enabled} aria-label={rule.name} disabled={busy}
              onclick={() => act(() => api.ruleEnable(rule.id, !rule.enabled))}><span class="toggle-knob"></span></button>
            <div class="rule-copy">
              <span class="rule-name">{rule.name}</span>
              <span class="muted clip">{summary(rule)}</span>
              {#if statOf(rule.id)}
                {@const s = statOf(rule.id)}
                <span class="stats">
                  {#if s?.sent}<span class="tag" data-outcome="sent">{$t("phonechat.rules.outcome.sent")} {s.sent}</span>{/if}
                  {#if s?.skipped}<span class="tag">{$t("phonechat.rules.outcome.skipped")} {s.skipped}</span>{/if}
                  {#if s?.failed}<span class="tag" data-outcome="failed">{$t("phonechat.rules.outcome.failed")} {s.failed}</span>{/if}
                </span>
              {/if}
            </div>
            {#if entryLink(rule)}
              <CopyButton text={() => entryLink(rule) ?? ""} label={$t("phonechat.rules.copy_link") as string} doneLabel={$t("phonechat.rules.copied") as string} errorLabel={$t("phonechat.rules.copy_failed") as string} />
            {/if}
            <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.rules.edit")} title={$t("phonechat.rules.edit")} onclick={() => edit(rule)}><WaIcon name="edit" /></button>
            <button class="btn btn-ghost btn-icon btn-sm" type="button" aria-label={$t("phonechat.rules.delete")} title={$t("phonechat.rules.delete")} disabled={busy} onclick={() => act(() => api.ruleDelete(rule.id))}><WaIcon name="trash" /></button>
          </li>
        {/each}
      </ul>
    {/if}
    <button class="btn btn-secondary btn-sm add" type="button" onclick={() => ((form = blank()), (error = null))}><WaIcon name="plus" /> {$t("phonechat.rules.add")}</button>

    {#if view.rules.length}
      <form class="pair" onsubmit={test}>
        <input class="input grow" bind:value={probe} placeholder={$t("phonechat.rules.test_placeholder")} aria-label={$t("phonechat.rules.test")} oninput={() => (probed = null)} />
        <button class="btn btn-secondary" type="submit" disabled={!probe.trim()}>{$t("phonechat.rules.test")}</button>
      </form>
      {#if probed}
        <div class="probe" role="status">
          {#if probed.opt_out}
            {$t("phonechat.rules.test_optout")}
          {:else if !probed.rule}
            {$t("phonechat.rules.test_none")}
          {:else if probed.reply}
            <span class="muted">{$t("phonechat.rules.test_hit", { rule: probed.rule.name })}</span>
            <span class="probe-reply">{probed.reply}</span>
          {:else}
            {$t("phonechat.rules.test_ai", { rule: probed.rule.name })}
          {/if}
        </div>
      {/if}
    {/if}

    {#if steps.length}
      <details class="more" open>
        <summary>{$t("phonechat.rules.flow")}</summary>
        <ul class="plain flow">
          {#each steps as step (step.from.id)}
            <li>
              <span class="rule-name">{step.from.name}</span>
              <span class="arrow" aria-hidden="true">→</span>
              <span class="tag">{step.tag}</span>
              <span class="arrow" aria-hidden="true">→</span>
              <span class="clip">{step.next.map((n) => n.name).join(", ")}</span>
            </li>
          {/each}
        </ul>
      </details>
    {/if}

    <details class="more">
      <summary>{$t("phonechat.rules.limits")}</summary>
      <div class="limits">
        <label class="field"><span>{$t("phonechat.rules.per_hour")}</span>
          <input class="input" type="number" min="1" max="120" value={view.limits.per_hour} onchange={(e) => setLimits({ per_hour: numberOf(e, 30) })} /></label>
        <label class="field"><span>{$t("phonechat.rules.per_day")}</span>
          <input class="input" type="number" min="1" max="1000" value={view.limits.per_day} onchange={(e) => setLimits({ per_day: numberOf(e, 200) })} /></label>
        <label class="field"><span>{$t("phonechat.rules.pause")}</span>
          <input class="input" type="number" min="0" max="1440" value={view.limits.manual_pause_minutes} onchange={(e) => setLimits({ manual_pause_minutes: Math.max(0, Math.round(Number((e.target as HTMLInputElement).value) || 0)) })} /></label>
      </div>
    </details>

    {#if view.opt_outs.length}
      <details class="more">
        <summary>{$t("phonechat.rules.optouts")} ({view.opt_outs.length})</summary>
        <ul class="plain">
          {#each view.opt_outs as o (o.chat)}
            <li><span class="clip">{o.name}</span>
              <button class="btn btn-ghost btn-sm" type="button" disabled={busy} onclick={() => act(() => api.optIn(o.chat))}>{$t("phonechat.rules.optout_remove")}</button></li>
          {/each}
        </ul>
      </details>
    {/if}

    <details class="more" open={runs.length > 0}>
      <summary>{$t("phonechat.rules.log")}</summary>
      {#if runs.length === 0}
        <p class="muted">{$t("phonechat.rules.log_empty")}</p>
      {:else}
        <ul class="plain log">
          {#each runs as run (run.id)}
            <li>
              <span class="tag" data-outcome={run.outcome}>{$t(`phonechat.rules.outcome.${run.outcome}`)}</span>
              <span class="clip"><strong>{run.chat_name}</strong> · {ruleName(run.rule_id)}{#if detail(run)} · {detail(run)}{/if}</span>
              <span class="muted nowrap">{when(run.ts)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </details>
    {#if error}<p class="err" role="alert">{errorLabel(error)}</p>{/if}
  {/if}
</WaModal>

<style>
  .center {
    display: flex;
    justify-content: center;
    padding: var(--space-6);
  }

  .editor,
  .limits {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .field small {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .field.narrow {
    max-width: 320px;
  }

  .area {
    resize: vertical;
    font: inherit;
    padding-block: 8px;
  }

  .pair,
  .row,
  .master {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pair {
    margin-top: var(--space-3);
  }

  .editor .pair {
    margin-top: 0;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .row.end {
    justify-content: flex-end;
  }

  .master {
    justify-content: space-between;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }

  .master-copy,
  .rule-copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .master-title,
  .rule-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .stats {
    display: flex;
    gap: 4px;
    margin-top: 4px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ask {
    align-items: flex-end;
  }

  .follow {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .follow legend {
    padding: 0 0 var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .mins {
    width: 88px;
  }

  .add {
    align-self: flex-start;
  }

  .muted {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .clip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .nowrap {
    white-space: nowrap;
    flex-shrink: 0;
  }

  .risk {
    display: flex;
    gap: var(--space-2);
    margin: var(--space-2) 0 var(--space-3);
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text-dim);
  }

  .risk :global(.wa-icon) {
    flex-shrink: 0;
    margin-top: 2px;
    color: var(--warning);
  }

  .empty {
    padding: var(--space-5) var(--space-3);
    text-align: center;
  }

  .rules,
  .plain {
    list-style: none;
    margin: 0 0 var(--space-3);
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .rules li,
  .plain li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    border-bottom: var(--hairline) solid var(--separator);
    min-width: 0;
  }

  .rules li.off .rule-copy {
    opacity: 0.55;
  }

  .plain li {
    justify-content: space-between;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .flow li {
    justify-content: flex-start;
  }

  .arrow {
    color: var(--text-muted);
  }

  :global([dir="rtl"]) .arrow {
    transform: scaleX(-1);
  }

  .log li {
    justify-content: flex-start;
  }

  .log .clip {
    flex: 1;
  }

  .tag {
    flex-shrink: 0;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .tag[data-outcome="sent"] {
    color: var(--success);
  }

  .tag[data-outcome="failed"] {
    color: var(--warning);
  }

  .probe {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .probe-reply {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .more {
    margin-top: var(--space-3);
    font-size: var(--text-sm);
  }

  .more summary {
    cursor: pointer;
    color: var(--text-dim);
    padding: var(--space-1) 0;
  }

  .limits {
    padding-top: var(--space-2);
  }

  .err {
    margin: var(--space-2) 0 0;
    font-size: var(--text-xs);
    color: var(--warning);
  }
</style>
