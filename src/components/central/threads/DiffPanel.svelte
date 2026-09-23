<script lang="ts">
  // Diff surface: one turn (checkpoint N-1 → N) or the whole thread (base →
  // working tree). Falls back to the driver-reported diff of the turn when no
  // checkpoint exists. Line selections become review chips in the composer;
  // "Revert to here" opens the edit-from-here dialog.
  import { t } from "$lib/i18n";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import type { DiffResult, FileDiff, ReviewComment } from "$lib/central/vcs";
  import { compactCount, vcsError } from "$lib/central/vcs";
  import { threadDiff } from "$lib/central/threads-ui/api";
  import Icon from "./Icon.svelte";

  let {
    threadId,
    cwd,
    turns,
    scope,
    turn,
    reveal = null,
    fallback = null,
    onscope,
    oncomment,
    onrevert,
  }: {
    threadId: string;
    cwd: string | null;
    /** Settled turn ordinals, oldest first. */
    turns: number[];
    scope: "turn" | "total";
    turn: number | null;
    reveal?: string | null;
    /** Driver diff for the selected turn when no checkpoint exists. */
    fallback?: FileDiff[] | null;
    onscope: (scope: "turn" | "total", turn: number | null) => void;
    oncomment: (c: ReviewComment) => void;
    onrevert: (turnOrdinal: number) => void;
  } = $props();

  let result = $state<DiffResult | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let mode = $state<"unified" | "split">("unified");
  let ignoreWs = $state(false);
  let seq = 0;

  let selectedTurn = $derived(turn ?? turns.at(-1) ?? null);

  async function load() {
    if (!cwd) return;
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const r = await threadDiff(
        threadId,
        cwd,
        scope === "total" || selectedTurn == null ? { kind: "total" } : { kind: "turn", turn: selectedTurn },
        { ignore_whitespace: ignoreWs },
      );
      if (my === seq) result = r;
    } catch (e) {
      if (my === seq) {
        result = null;
        error = vcsError(e).message;
      }
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    threadId;
    scope;
    selectedTurn;
    ignoreWs;
    cwd;
    void load();
  });

  let files = $derived<FileDiff[]>(result?.files ?? (error && scope === "turn" && fallback?.length ? fallback : []));
  let adds = $derived(files.reduce((n, f) => n + f.additions, 0));
  let dels = $derived(files.reduce((n, f) => n + f.deletions, 0));
</script>

<div class="panel-diff">
  <div class="toolbar">
    <div class="seg" role="radiogroup" aria-label={$t("llm.central.threads.diff.scope")}>
      <button type="button" role="radio" aria-checked={scope === "turn"} class:on={scope === "turn"} disabled={!turns.length} onclick={() => onscope("turn", selectedTurn)}>
        {$t("llm.central.threads.diff.turn")}
      </button>
      <button type="button" role="radio" aria-checked={scope === "total"} class:on={scope === "total"} onclick={() => onscope("total", selectedTurn)}>
        {$t("llm.central.threads.diff.total")}
      </button>
    </div>
    {#if scope === "turn" && turns.length}
      <select aria-label={$t("llm.central.threads.diff.pick_turn")} value={selectedTurn ?? ""} onchange={(e) => onscope("turn", Number(e.currentTarget.value))}>
        {#each [...turns].reverse() as n (n)}<option value={n}>{$t("llm.central.threads.diff.turn_n", { n })}</option>{/each}
      </select>
    {/if}
    <span class="stats">{#if files.length}<span class="add">+{compactCount(adds)}</span> <span class="del">−{compactCount(dels)}</span>{/if}</span>
    <span class="grow"></span>
    <label class="ws" title={$t("llm.central.threads.diff.ignore_ws")}>
      <input type="checkbox" bind:checked={ignoreWs} />{$t("llm.central.threads.diff.ws")}
    </label>
    <button type="button" class="ib" aria-label={$t("llm.central.threads.refresh")} title={$t("llm.central.threads.refresh")} onclick={load}><Icon name="arrow-counter-clockwise" size={13} /></button>
    {#if scope === "turn" && selectedTurn != null}
      <button type="button" class="ib labeled" title={$t("llm.central.threads.diff.revert_hint")} onclick={() => onrevert(selectedTurn!)}>
        <Icon name="arrow-bend-down-left" size={13} />{$t("llm.central.threads.diff.revert")}
      </button>
    {/if}
  </div>

  {#if !cwd}
    <p class="note">{$t("llm.central.threads.diff.no_folder")}</p>
  {:else if error && !files.length}
    <p class="note">{$t("llm.central.threads.diff.unavailable")}<br /><span class="muted">{error}</span></p>
  {:else}
    <div class="viewer" aria-busy={loading}>
      <DiffViewer {files} bind:mode height="100%" revealPath={reveal} {oncomment} emptyText={loading ? ($t("llm.central.vcs.loading") as string) : null} />
    </div>
  {/if}
</div>

<style>
  .panel-diff { display: flex; flex-direction: column; min-height: 0; height: 100%; }
  .toolbar { display: flex; align-items: center; gap: 6px; padding: 8px 10px; border-bottom: 1px solid var(--separator); flex-wrap: wrap; }
  .seg { display: inline-flex; padding: 2px; border-radius: 8px; background: var(--fill-1); }
  .seg button { border: 0; background: transparent; color: var(--text-muted); font-size: 12px; padding: 4px 10px; border-radius: 6px; cursor: pointer; }
  .seg button.on { background: var(--surface, #fff); color: var(--text); font-weight: 600; box-shadow: 0 1px 2px color-mix(in srgb, #000 12%, transparent); }
  select { font-size: 12px; border: 1px solid var(--separator); background: var(--surface); color: var(--text); border-radius: 6px; height: 26px; }
  .stats { font-family: var(--font-mono); font-size: 11.5px; }
  .add { color: var(--green, #22c55e); }
  .del { color: var(--red, #ef4444); }
  .grow { flex: 1; }
  .ws { display: inline-flex; align-items: center; gap: 4px; font-size: 11.5px; color: var(--text-muted); cursor: pointer; }
  .ib { display: inline-flex; align-items: center; gap: 5px; border: 0; background: transparent; color: var(--text-muted); padding: 4px 6px; border-radius: 6px; cursor: pointer; font-size: 12px; }
  .ib:hover { background: var(--fill-2); color: var(--text); }
  .viewer { flex: 1; min-height: 0; }
  .note { margin: 24px 16px; color: var(--text-muted); font-size: 13px; text-align: center; }
  .muted { font-size: 11.5px; font-family: var(--font-mono); }
</style>
