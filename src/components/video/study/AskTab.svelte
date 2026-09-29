<script lang="ts">
  // Ask the AI about this video. Answers come from the transcript, and every
  // [mm:ss] citation is a button that seeks there.
  import { t } from "$lib/i18n";
  import * as api from "$lib/video/api";
  import { splitCitations } from "$lib/video/format";
  import { player, seek } from "$lib/video/player.svelte";
  import VIcon from "$components/video/player/VIcon.svelte";

  type Mode = "ask" | "summary" | "quiz" | "chapters";
  type Turn = { id: number; mode: Mode; question: string; answer: string | null; error: string | null };

  let turns = $state<Turn[]>([]);
  let question = $state("");
  let busy = $state(false);
  let forKey = $state<string | null>(null);
  let listEl: HTMLElement | undefined = $state();
  let nextId = 0;

  $effect(() => {
    const key = player.info?.key ?? null;
    if (key !== forKey) {
      forKey = key;
      turns = [];
    }
  });

  async function run(mode: Mode, q = "") {
    const key = player.info?.key;
    if (!key || busy) return;
    const turn: Turn = { id: ++nextId, mode, question: q, answer: null, error: null };
    turns = [...turns, turn];
    busy = true;
    scrollEnd();
    try {
      const answer = await api.ask(key, q, mode);
      turns = turns.map((x) => (x.id === turn.id ? { ...x, answer } : x));
    } catch (e) {
      turns = turns.map((x) => (x.id === turn.id ? { ...x, error: String(e) } : x));
    } finally {
      busy = false;
      scrollEnd();
    }
  }

  function submit(e: Event) {
    e.preventDefault();
    const q = question.trim();
    if (!q) return;
    question = "";
    void run("ask", q);
  }

  function scrollEnd() {
    queueMicrotask(() => listEl?.scrollTo({ top: listEl.scrollHeight, behavior: "smooth" }));
  }
</script>

<div class="ask">
  <div class="list" bind:this={listEl}>
    {#if !turns.length}
      <div class="intro">
        <img src="/icons/video/ask-ai-64.webp" alt="" width="44" height="44" />
        <p>{$t("video.ask.intro")}</p>
      </div>
    {/if}
    {#each turns as turn (turn.id)}
      <div class="q">{turn.mode === "ask" ? turn.question : $t(`video.ask.mode_${turn.mode}`)}</div>
      <div class="a">
        {#if turn.error}
          <p class="err">{turn.error}</p>
        {:else if turn.answer == null}
          <span class="thinking"><VIcon name="loader" size={13} spin /> {$t("video.ask.thinking")}</span>
        {:else}
          <p class="ans">
            {#each splitCitations(turn.answer) as part, i (i)}
              {#if "t_ms" in part}
                <button class="cite" onclick={() => seek(part.t_ms / 1000)}>{part.label}</button>
              {:else}{part.text}{/if}
            {/each}
          </p>
        {/if}
      </div>
    {/each}
  </div>

  <div class="modes">
    <button class="chip" disabled={busy} onclick={() => run("summary")}><VIcon name="notes" size={12} /> {$t("video.ask.mode_summary")}</button>
    <button class="chip" disabled={busy} onclick={() => run("quiz")}><VIcon name="cards" size={12} /> {$t("video.ask.mode_quiz")}</button>
    <button class="chip" disabled={busy} onclick={() => run("chapters")}><VIcon name="list" size={12} /> {$t("video.ask.mode_chapters")}</button>
  </div>
  <form class="input" onsubmit={submit}>
    <textarea
      rows="1"
      bind:value={question}
      placeholder={$t("video.ask.placeholder") as string}
      aria-label={$t("video.ask.placeholder") as string}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.shiftKey) submit(e);
      }}
    ></textarea>
    <button type="submit" class="send" disabled={busy || !question.trim()} aria-label={$t("video.ask.send") as string}>
      <VIcon name="send" size={14} />
    </button>
  </form>
</div>

<style>
  .ask {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 12px;
  }
  .intro {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    margin: auto 0;
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .intro p {
    margin: 0;
    max-width: 260px;
  }
  .q {
    align-self: flex-end;
    max-width: 85%;
    padding: 7px 11px;
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-xs) var(--radius-lg);
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--text-sm);
    white-space: pre-wrap;
  }
  .a {
    align-self: flex-start;
    max-width: 100%;
    padding: 8px 11px;
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-lg) var(--radius-xs);
    background: var(--fill-1);
    font-size: var(--text-sm);
    line-height: var(--leading-base);
  }
  .ans {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .err {
    margin: 0;
    color: var(--danger);
  }
  .thinking {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
  }
  .cite {
    all: unset;
    display: inline-block;
    padding: 0 5px;
    margin: 0 1px;
    border-radius: var(--radius-xs);
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-xs);
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .cite:hover {
    background: var(--accent-glow);
  }
  .cite:focus-visible,
  .chip:focus-visible,
  .send:focus-visible {
    outline: var(--focus-ring);
  }
  .modes {
    display: flex;
    gap: 6px;
    padding: 6px 12px 0;
    flex-wrap: wrap;
  }
  .chip {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .chip:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .chip:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .input {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    margin: 8px 12px 12px;
    padding: 6px 6px 6px 12px;
    border-radius: var(--radius-xl);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
  }
  .input:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  textarea {
    flex: 1;
    min-height: 22px;
    max-height: 120px;
    resize: none;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    outline: none;
    field-sizing: content;
  }
  .send {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
