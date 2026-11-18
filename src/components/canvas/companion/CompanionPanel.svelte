<script lang="ts">
  import { onMount } from "svelte";
  import { locale, t } from "$lib/i18n";
  import {
    act,
    dismiss,
    errorText,
    getSettings,
    localModels,
    setSettings,
    summarize,
    askOmni,
    entrySuggestions,
    entrySummary,
    type AskAnswer,
    type CompanionEntry,
    type CompanionSettings,
    type ModelMode,
  } from "$lib/canvas/companion/api";
  import { companionFeed } from "$lib/canvas/companion/store.svelte";
  import { stateVar } from "$lib/canvas/state/agents.svelte";

  interface Props {
    workspaceId?: string;
    terminals?: Array<{ id: string; title: string }>;
    onFocusNode?: (nodeId: string) => void;
    onClose?: () => void;
    fill?: boolean;
  }
  let { workspaceId, terminals = [], onFocusNode, onClose, fill = false }: Props = $props();

  let settings = $state<CompanionSettings | null>(null);
  let models = $state<string[]>([]);
  let showSettings = $state(false);
  let busy = $state<string | null>(null);
  let error = $state("");
  let askNode = $state("");
  let question = $state("");
  let answer = $state<AskAnswer | null>(null);
  let sentId = $state<string | null>(null);

  const entries = $derived(companionFeed.forWorkspace(workspaceId));

  const KIND_STATE = { done: "success", blocked: "blocked", failed: "failed", manual: "idle" } as const;

  onMount(() => {
    void companionFeed.start();
    void (async () => {
      try {
        const s = await getSettings();
        const lang = ($locale as string | undefined) ?? null;
        settings = lang && s.language !== lang ? await setSettings({ ...s, language: lang }) : s;
      } catch (e) {
        error = errorText(e);
      }
    })();
    return () => companionFeed.stop();
  });

  async function save(patch: Partial<CompanionSettings>) {
    if (!settings) return;
    try {
      settings = await setSettings({ ...settings, ...patch });
    } catch (e) {
      error = errorText(e);
    }
  }

  async function openSettings() {
    showSettings = !showSettings;
    if (showSettings) models = await localModels().catch(() => []);
  }

  const tr = (k: string, v?: Record<string, unknown>) => $t(k, v) as string;

  async function send(entry: CompanionEntry, index: number) {
    busy = `${entry.id}:${index}`;
    error = "";
    try {
      const text = entrySuggestions(entry, tr)[index];
      await act(entry.id, index, entry.i18n ? text : undefined);
      sentId = entry.id;
      companionFeed.drop(entry.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = null;
    }
  }

  async function close(entry: CompanionEntry) {
    companionFeed.drop(entry.id);
    await dismiss(entry.id).catch(() => {});
  }

  async function ask() {
    if (!askNode) return;
    busy = `ask:${askNode}`;
    error = "";
    try {
      const e = await summarize(askNode);
      companionFeed.push(e);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = null;
    }
  }

  async function askFree(e?: SubmitEvent) {
    e?.preventDefault();
    const q = question.trim();
    if (!q || !workspaceId) return;
    busy = "free";
    error = "";
    try {
      answer = await askOmni(workspaceId, q, askNode || undefined);
      question = "";
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = null;
    }
  }

  function ago(ms: number): string {
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 60) return $t("canvas.companion.just_now", { default: "just now" }) as string;
    if (s < 3600) return `${Math.round(s / 60)} min`;
    return `${Math.round(s / 3600)} h`;
  }

  function kindLabel(kind: CompanionEntry["kind"]): string {
    const d = { done: "Finished", blocked: "Needs you", failed: "Failed", manual: "Checked" }[kind];
    return $t(`canvas.companion.kind.${kind}`, { default: d }) as string;
  }

  function sourceLabel(src: string): string {
    if (src.startsWith("ollama:")) return src.slice(7);
    if (src.startsWith("llm:")) return src.slice(4);
    if (src.startsWith("via ")) return src;
    if (src === "omni") return "Omni";
    return $t("canvas.companion.source.heuristic", { default: "no model" }) as string;
  }
</script>

<section class="panel" class:fill aria-label={$t("canvas.companion.title", { default: "Omni" }) as string}>
  <header>
    <h3>{$t("canvas.companion.title", { default: "Omni" })}</h3>
    <div class="row">
      <button type="button" class="ghost" aria-expanded={showSettings} onclick={openSettings}>
        {$t("canvas.companion.settings", { default: "Settings" })}
      </button>
      {#if onClose}
        <button type="button" class="ghost" aria-label={$t("canvas.companion.close", { default: "Close" }) as string} onclick={onClose}>✕</button>
      {/if}
    </div>
  </header>

  {#if showSettings && settings}
    <div class="form">
      <label class="check">
        <input type="checkbox" checked={settings.enabled} onchange={(e) => save({ enabled: e.currentTarget.checked })} />
        <span>{$t("canvas.companion.enabled", { default: "Watch the agents" })}</span>
      </label>
      <label class="check">
        <input type="checkbox" checked={settings.petBalloon} onchange={(e) => save({ petBalloon: e.currentTarget.checked })} />
        <span>{$t("canvas.companion.pet_balloon", { default: "Show in the floating Omni" })}</span>
      </label>
      <div class="row start wrap">
        <label class="check"><input type="checkbox" checked={settings.onDone} onchange={(e) => save({ onDone: e.currentTarget.checked })} /><span>{$t("canvas.companion.kind.done", { default: "Finished" })}</span></label>
        <label class="check"><input type="checkbox" checked={settings.onBlocked} onchange={(e) => save({ onBlocked: e.currentTarget.checked })} /><span>{$t("canvas.companion.kind.blocked", { default: "Needs you" })}</span></label>
        <label class="check"><input type="checkbox" checked={settings.onFailed} onchange={(e) => save({ onFailed: e.currentTarget.checked })} /><span>{$t("canvas.companion.kind.failed", { default: "Failed" })}</span></label>
      </div>
      <label>
        <span>{$t("canvas.companion.min_work", { default: "Speak after at least (seconds of work)" })}</span>
        <input type="number" min="5" max="3600" value={settings.minWorkSecs} onchange={(e) => save({ minWorkSecs: Number(e.currentTarget.value) || 60 })} />
      </label>
      <label>
        <span>{$t("canvas.companion.model", { default: "Model" })}</span>
        <select value={settings.modelMode} onchange={(e) => save({ modelMode: e.currentTarget.value as ModelMode })}>
          <option value="auto">{$t("canvas.companion.model_auto", { default: "Local first, then the cheap agent" })}</option>
          <option value="local">{$t("canvas.companion.model_local", { default: "Local only (Ollama)" })}</option>
          <option value="cheap">{$t("canvas.companion.model_cheap", { default: "Cheap agent only" })}</option>
          <option value="heuristic">{$t("canvas.companion.model_none", { default: "No model" })}</option>
        </select>
      </label>
      <label>
        <span>{$t("canvas.companion.local_model", { default: "Local model" })}</span>
        <select value={settings.ollamaModel ?? ""} onchange={(e) => save({ ollamaModel: e.currentTarget.value || null })}>
          <option value="">{$t("canvas.companion.smallest", { default: "Smallest installed" })}</option>
          {#each models as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        {#if !models.length}<small class="muted">{$t("canvas.companion.no_ollama", { default: "Ollama is not running or has no model: Omni uses a summary without a model." })}</small>{/if}
      </label>
      <label class="check">
        <input type="checkbox" checked={settings.cliFallback ?? true} onchange={(e) => save({ cliFallback: e.currentTarget.checked })} />
        <span>{$t("canvas.companion.cli_fallback", { default: "Without a local model, ask an installed agent CLI (cheapest model, no tools)" })}</span>
      </label>
      <label>
        <span>{$t("canvas.companion.cli_agent", { default: "Agent CLI" })}</span>
        <select value={settings.cliAgent ?? ""} onchange={(e) => save({ cliAgent: e.currentTarget.value || null })}>
          <option value="">{$t("canvas.companion.cli_auto", { default: "The one used on the canvas" })}</option>
          <option value="claude">Claude (Haiku)</option>
          <option value="codex">Codex</option>
          <option value="gemini">Gemini (Flash)</option>
        </select>
      </label>
      <label>
        <span>{$t("canvas.companion.cheap_agent", { default: "Cheap agent id (optional)" })}</span>
        <input type="text" value={settings.cheapAgentId ?? ""} placeholder="agent id" onchange={(e) => save({ cheapAgentId: e.currentTarget.value || null })} />
      </label>
    </div>
  {/if}

  {#if terminals.length}
    <div class="row">
      <select bind:value={askNode} aria-label={$t("canvas.companion.ask_pick", { default: "Agent" }) as string}>
        <option value="">{$t("canvas.companion.ask_pick", { default: "Agent" })}</option>
        {#each terminals as n (n.id)}<option value={n.id}>{n.title}</option>{/each}
      </select>
      <button type="button" disabled={!askNode || busy === `ask:${askNode}`} onclick={ask}>
        {busy === `ask:${askNode}` ? $t("canvas.companion.thinking", { default: "Reading…" }) : $t("canvas.companion.ask", { default: "Ask Omni" })}
      </button>
    </div>
  {/if}

  {#if workspaceId}
    <form class="row" onsubmit={askFree}>
      <input
        type="text"
        class="question"
        bind:value={question}
        placeholder={$t("canvas.companion.question_placeholder", { default: "Ask about this workspace: what is B doing? note that…" }) as string}
        aria-label={$t("canvas.companion.question", { default: "Question for Omni" }) as string}
      />
      <button type="submit" disabled={!question.trim() || busy === "free"}>
        {busy === "free" ? $t("canvas.companion.thinking", { default: "Reading…" }) : $t("canvas.companion.ask_free", { default: "Ask" })}
      </button>
    </form>
  {/if}
  {#if answer}
    <article class="entry answer" role="status">
      <p class="answer-text">{answer.answer}</p>
      {#if answer.sources.length}
        <div class="row">
          {#each answer.sources as id (id)}
            <button type="button" class="name" onclick={() => onFocusNode?.(id)}>{terminals.find((x) => x.id === id)?.title ?? id.slice(0, 6)}</button>
          {/each}
        </div>
      {/if}
      <p class="muted">{sourceLabel(answer.source)}</p>
    </article>
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if sentId}<p class="muted" role="status">{$t("canvas.companion.sent", { default: "Sent to the agent." })}</p>{/if}

  {#each entries as e (e.id)}
    <article class="entry" style="--k: {stateVar(KIND_STATE[e.kind])}">
      <div class="row">
        <button type="button" class="name" onclick={() => onFocusNode?.(e.nodeId)}>{e.name}</button>
        <span class="badge">{kindLabel(e.kind)}</span>
        <span class="muted">{ago(e.atMs)}</span>
        <button type="button" class="ghost x" aria-label={$t("canvas.companion.dismiss", { default: "Dismiss" }) as string} onclick={() => close(e)}>✕</button>
      </div>
      <p class="summary">{entrySummary(e, tr)}</p>
      <div class="suggestions">
        {#each entrySuggestions(e, tr) as s, i (i)}
          <button type="button" class="sugg" disabled={busy === `${e.id}:${i}`} onclick={() => send(e, i)}>{s}</button>
        {/each}
      </div>
      <small class="muted">{sourceLabel(e.source)}</small>
    </article>
  {:else}
    <p class="muted empty">{$t("canvas.companion.empty", { default: "Omni speaks up when an agent finishes a real piece of work, gets stuck or fails." })}</p>
  {/each}
</section>

<style>
  .panel.fill {
    width: auto;
    max-height: none;
    height: 100%;
    border: none;
    border-radius: 0;
    box-shadow: none;
    background: transparent;
  }
  .panel {
    width: min(380px, calc(100vw - 32px));
    max-height: 75vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.3);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  header,
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .row.start {
    justify-content: flex-start;
  }
  .row.wrap {
    flex-wrap: wrap;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  label > span {
    color: var(--text-muted);
    font-size: 12px;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  input:not([type="checkbox"]),
  select {
    font: inherit;
    color: var(--text);
    background: var(--input-bg, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 5px 8px;
    min-width: 0;
    flex: 1;
  }
  button {
    font: inherit;
    border: 0;
    border-radius: 8px;
    padding: 5px 10px;
    background: var(--accent);
    color: #fff;
    cursor: default;
  }
  button:disabled {
    opacity: 0.5;
  }
  button.ghost {
    background: transparent;
    color: var(--text-muted);
    border: 1px solid var(--border);
  }
  button.x {
    border: 0;
    padding: 2px 6px;
  }
  .entry {
    border: 1px solid var(--border);
    border-left: 3px solid var(--k);
    border-radius: 10px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .name {
    background: transparent;
    color: var(--text);
    font-weight: 600;
    padding: 0;
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 99px;
    color: var(--k);
    border: 1px solid var(--k);
  }
  .summary {
    margin: 0;
    line-height: 1.4;
  }
  .suggestions {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sugg {
    background: transparent;
    color: var(--text);
    border: 1px solid var(--border);
    text-align: left;
  }
  .sugg:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .empty {
    margin: 4px 0;
  }
  .error {
    color: #d96a62;
    margin: 0;
  }
  .question {
    flex: 1;
    min-width: 0;
  }
  .answer-text {
    margin: 0;
    white-space: pre-wrap;
  }
</style>
