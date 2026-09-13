<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { awardXp, bumpCounter } from "$lib/study-gamification";
  import PageHero from "$lib/study-components/PageHero.svelte";

  interface Props {
    deckName?: string | null;
    deckId?: number | null;
  }

  let { deckName = null, deckId = null }: Props = $props();

  type CardSummary = { id: number };
  type SearchResponse = { items: CardSummary[]; total: number };

  type FieldConfig = Record<string, unknown>;
  type Field = { ord: number; name: string; config: FieldConfig };
  type TemplateConfig = {
    q_format: string;
    a_format: string;
    [k: string]: unknown;
  };
  type Template = { ord: number; name: string; config: TemplateConfig };
  type Notetype = {
    id: number;
    name: string;
    config: { kind: string; css: string; sort_field_idx: number; [k: string]: unknown };
    fields: Field[];
    templates: Template[];
  };
  type Note = {
    id: number;
    mid: number;
    fields: string[];
    tags: string[];
  };
  type Card = {
    id: number;
    nid: number;
    did: number;
    ord: number;
    queue: string;
    ctype: string;
    ivl: number;
    reps: number;
    lapses: number;
  };

  type Preview = {
    again_ivl_days: number;
    hard_ivl_days: number;
    good_ivl_days: number;
    easy_ivl_days: number;
    again_due_secs: number;
    hard_due_secs: number;
    good_due_secs: number;
    easy_due_secs: number;
  };

  let loading = $state(true);
  let error = $state("");
  let queue = $state<number[]>([]);
  let queueIndex = $state(0);
  let initialTotal = $state(0);
  let answered = $state(0);

  let card = $state<Card | null>(null);
  let note = $state<Note | null>(null);
  let notetype = $state<Notetype | null>(null);
  let preview = $state<Preview | null>(null);
  let showAnswer = $state(false);
  let busyAnswer = $state(false);

  function buildSearchQuery(): string {
    const parts: string[] = ["is:due"];
    if (deckName) parts.unshift(`"deck:${deckName}"`);
    return parts.join(" ");
  }

  async function loadQueue() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const q = buildSearchQuery();
      const res = await pluginInvoke<SearchResponse>("study", "study:anki:search:cards", {
        query: q,
        limit: 1000,
        offset: 0,
      });
      queue = res.items.map((c) => c.id);
      initialTotal = queue.length;
      queueIndex = 0;
      answered = 0;
      await loadCurrent();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function loadCurrent() {
    showAnswer = false;
    preview = null;
    if (queueIndex >= queue.length) {
      card = null;
      note = null;
      notetype = null;
      return;
    }
    const cid = queue[queueIndex];
    try {
      const c = await pluginInvoke<Card>("study", "study:anki:cards:get", { id: cid });
      const n = await pluginInvoke<Note>("study", "study:anki:notes:get", { id: c.nid });
      const nt = await pluginInvoke<Notetype>("study", "study:anki:notetypes:get", {
        id: n.mid,
      });
      card = c;
      note = n;
      notetype = nt;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function loadPreview() {
    if (!card) return;
    try {
      const p = await pluginInvoke<Preview>("study", "study:anki:cards:preview_states", {
        cardId: card.id,
      });
      preview = p;
    } catch (e) {
      console.error("preview failed", e);
    }
  }

  function renderTemplate(tpl: string, frontHtml: string | null): string {
    if (!note || !notetype) return "";
    let out = tpl;
    for (let i = 0; i < notetype.fields.length; i++) {
      const f = notetype.fields[i];
      const v = note.fields[i] ?? "";
      const safeName = f.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const blockOpen = new RegExp(`{{#${safeName}}}([\\s\\S]*?){{/${safeName}}}`, "g");
      out = out.replace(blockOpen, (_m, inner) => (v.trim() ? inner : ""));
      const blockNeg = new RegExp(`{{\\^${safeName}}}([\\s\\S]*?){{/${safeName}}}`, "g");
      out = out.replace(blockNeg, (_m, inner) => (v.trim() ? "" : inner));
      const fieldRe = new RegExp(`{{${safeName}}}`, "g");
      out = out.replace(fieldRe, v);
      const clozeRe = new RegExp(`{{cloze:${safeName}}}`, "g");
      out = out.replace(clozeRe, v.replace(/\{\{c\d+::([^}|]+)(?:::[^}]*)?\}\}/g, "[…]"));
    }
    if (frontHtml !== null) {
      out = out.replace(/{{FrontSide}}/g, frontHtml);
    }
    return out;
  }

  function buildSrcdoc(html: string, css: string): string {
    return `<!doctype html><html><head><meta charset="utf-8"><style>
      html,body{margin:0;padding:16px;background:transparent;color:inherit;font-family:inherit;font-size:16px;line-height:1.5;}
      ${css}
    </style></head><body class="card">${html}</body></html>`;
  }

  const frontHtml = $derived.by(() => {
    if (!card || !note || !notetype) return "";
    const tpl = notetype.templates[card.ord]?.config.q_format ?? "";
    return renderTemplate(tpl, null);
  });

  const backHtml = $derived.by(() => {
    if (!card || !note || !notetype) return "";
    const tpl = notetype.templates[card.ord]?.config.a_format ?? "";
    return renderTemplate(tpl, frontHtml);
  });

  const cssBlock = $derived(notetype?.config.css ?? "");

  const frontDoc = $derived(buildSrcdoc(frontHtml, cssBlock));
  const backDoc = $derived(buildSrcdoc(backHtml, cssBlock));

  async function reveal() {
    if (showAnswer || !card || busyAnswer) return;
    showAnswer = true;
    await loadPreview();
  }

  async function answer(button: 1 | 2 | 3 | 4) {
    if (!card || busyAnswer || !showAnswer) return;
    busyAnswer = true;
    try {
      await pluginInvoke("study", "study:anki:cards:answer", {
        cardId: card.id,
        button,
      });
      answered += 1;
      queueIndex += 1;
      if (button > 1) {
        void awardXp("anki_review", 5, { card_id: card.id, button });
      }
      void bumpCounter("anki_reviews", 1);
      await loadCurrent();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busyAnswer = false;
    }
  }

  function fmtIvl(days: number): string {
    if (days < 1) {
      const mins = Math.max(1, Math.round(days * 24 * 60));
      return mins < 60 ? `<${mins}m` : `<${Math.round(mins / 60)}h`;
    }
    if (days < 30) return `${Math.round(days)}d`;
    if (days < 365) return `${Math.round(days / 30)}mo`;
    return `${Math.round(days / 365)}y`;
  }

  function onKey(e: KeyboardEvent) {
    if (loading || error) return;
    const target = e.target as HTMLElement;
    if (target?.tagName === "INPUT" || target?.tagName === "TEXTAREA") return;
    if (!showAnswer) {
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        reveal();
      }
      return;
    }
    if (e.key === "1") {
      e.preventDefault();
      answer(1);
    } else if (e.key === "2") {
      e.preventDefault();
      answer(2);
    } else if (e.key === "3" || e.key === " " || e.key === "Enter") {
      e.preventDefault();
      answer(3);
    } else if (e.key === "4") {
      e.preventDefault();
      answer(4);
    }
  }

  onMount(() => {
    loadQueue();
    window.addEventListener("keydown", onKey);
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKey);
  });

  const progressPct = $derived(
    initialTotal === 0 ? 0 : Math.round((answered / initialTotal) * 100),
  );

  const eyebrow = $derived(deckName ? $t("study.anki.review.eyebrow_deck", { name: deckName }) : $t("study.anki.review.eyebrow_all"));
</script>

<section class="study-page">
  <PageHero title={$t("study.anki.review.title")} subtitle={eyebrow} />

  {#if loading}
    <p class="muted">{$t("study.anki.review.loading_session")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if !card}
    <section class="card complete-card">
      <div class="complete-icon" aria-hidden="true">✓</div>
      <h2>{$t("study.anki.review.session_complete")}</h2>
      {#if initialTotal === 0}
        <p>{$t("study.anki.review.no_pending")}</p>
      {:else}
        <p>Você respondeu {answered} cards. Bom trabalho!</p>
      {/if}
      <div class="complete-actions">
        <a class="btn-primary" href="/study/anki">{$t("study.anki.sidebar.dashboard")}</a>
        {#if initialTotal > 0}
          <button type="button" class="btn-secondary" onclick={loadQueue}>
            Tentar mais cards
          </button>
        {/if}
      </div>
    </section>
  {:else}
    <header class="progress-row">
      <div class="progress-track" aria-hidden="true">
        <div class="progress-fill" style:width="{progressPct}%"></div>
      </div>
      <span class="progress-text">{answered} / {initialTotal}</span>
    </header>

    <article class="card-stage">
      <iframe
        title={showAnswer ? $t("study.anki.review.answer") : $t("study.anki.review.question")}
        srcdoc={showAnswer ? backDoc : frontDoc}
        sandbox="allow-same-origin"
        class="card-frame"
      ></iframe>
    </article>

    {#if !showAnswer}
      <div class="cta-row">
        <button type="button" class="btn-primary big" onclick={reveal}>
          {$t("study.anki.review.show_answer")}
        </button>
        <span class="kbd-hint">{$t("study.anki.review.space")}</span>
      </div>
    {:else}
      <div class="rating-row">
        <button
          type="button"
          class="rate-btn again"
          onclick={() => answer(1)}
          disabled={busyAnswer}
        >
          <span class="rate-label">{$t("study.anki.revlog.ease_again")}</span>
          <span class="rate-ivl">{preview ? fmtIvl(preview.again_ivl_days) : "—"}</span>
          <span class="rate-key">1</span>
        </button>
        <button
          type="button"
          class="rate-btn hard"
          onclick={() => answer(2)}
          disabled={busyAnswer}
        >
          <span class="rate-label">{$t("study.anki.revlog.ease_hard")}</span>
          <span class="rate-ivl">{preview ? fmtIvl(preview.hard_ivl_days) : "—"}</span>
          <span class="rate-key">2</span>
        </button>
        <button
          type="button"
          class="rate-btn good"
          onclick={() => answer(3)}
          disabled={busyAnswer}
        >
          <span class="rate-label">Bom</span>
          <span class="rate-ivl">{preview ? fmtIvl(preview.good_ivl_days) : "—"}</span>
          <span class="rate-key">3</span>
        </button>
        <button
          type="button"
          class="rate-btn easy"
          onclick={() => answer(4)}
          disabled={busyAnswer}
        >
          <span class="rate-label">{$t("study.anki.revlog.ease_easy")}</span>
          <span class="rate-ivl">{preview ? fmtIvl(preview.easy_ivl_days) : "—"}</span>
          <span class="rate-key">4</span>
        </button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    max-width: 900px;
    margin-inline: auto;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
    text-align: center;
    padding: calc(var(--padding) * 3);
  }
  .error {
    color: var(--error);
    font-size: 13px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--error) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 25%, var(--input-border));
    border-radius: var(--border-radius);
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .progress-track {
    flex: 1;
    height: 4px;
    background: color-mix(in oklab, var(--input-border) 70%, transparent);
    border-radius: 2px;
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 300ms ease-out;
  }
  .progress-text {
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: var(--tertiary);
    min-width: 60px;
    text-align: right;
  }

  .card-stage {
    display: flex;
    flex-direction: column;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    min-height: 320px;
  }
  .card-frame {
    width: 100%;
    min-height: 320px;
    border: 0;
    background: transparent;
  }

  .cta-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--padding);
  }
  .kbd-hint {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    color: var(--tertiary);
    padding: 2px 8px;
    border: none;
    border-radius: 6px;
    background: var(--bg);
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 10px 24px;
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border: 0;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    text-decoration: none;
    transition: filter 150ms ease;
  }
  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn-primary.big {
    padding: 12px 36px;
    font-size: 15px;
  }
  .btn-primary:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .btn-secondary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 10px 20px;
    background: transparent;
    border: none;
    color: var(--secondary);
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 14px;
    cursor: pointer;
    transition: border-color 150ms ease;
  }
  .btn-secondary:hover {
    border-color: var(--accent);
  }
  .btn-secondary:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .rating-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .rate-btn {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 14px 8px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    transition:
      border-color 150ms ease,
      background 150ms ease,
      transform 100ms ease;
  }
  .rate-btn:hover:not(:disabled) {
    transform: translateY(-1px);
  }
  .rate-btn:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .rate-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .rate-btn.again {
    border-color: color-mix(in oklab, var(--error) 35%, var(--input-border));
  }
  .rate-btn.again:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error) 10%, transparent);
  }
  .rate-btn.hard {
    border-color: color-mix(in oklab, var(--warning, var(--accent)) 35%, var(--input-border));
  }
  .rate-btn.hard:hover:not(:disabled) {
    background: color-mix(in oklab, var(--warning, var(--accent)) 10%, transparent);
  }
  .rate-btn.good {
    border-color: color-mix(in oklab, var(--accent) 35%, var(--input-border));
  }
  .rate-btn.good:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .rate-btn.easy {
    border-color: color-mix(in oklab, var(--success, var(--accent)) 35%, var(--input-border));
  }
  .rate-btn.easy:hover:not(:disabled) {
    background: color-mix(in oklab, var(--success, var(--accent)) 10%, transparent);
  }
  .rate-label {
    font-size: 13px;
    font-weight: 500;
  }
  .rate-ivl {
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: var(--tertiary);
  }
  .rate-key {
    position: absolute;
    top: 6px;
    right: 8px;
    font-family: var(--font-mono, monospace);
    font-size: 10px;
    color: var(--tertiary);
    padding: 1px 5px;
    border: none;
    border-radius: 4px;
  }

  .complete-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 4) calc(var(--padding) * 2);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    text-align: center;
  }
  .complete-icon {
    width: 56px;
    height: 56px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: color-mix(in oklab, var(--success, var(--accent)) 18%, transparent);
    color: var(--success, var(--accent));
    font-size: 28px;
  }
  .complete-card h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 500;
    color: var(--secondary);
  }
  .complete-card p {
    margin: 0;
    color: var(--tertiary);
    font-size: 14px;
  }
  .complete-actions {
    display: flex;
    gap: 10px;
    margin-top: 8px;
    flex-wrap: wrap;
    justify-content: center;
  }

  @media (max-width: 600px) {
    .rating-row {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .progress-fill,
    .rate-btn,
    .btn-primary,
    .btn-secondary {
      transition: none;
    }
  }
</style>
