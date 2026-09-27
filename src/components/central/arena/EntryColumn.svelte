<script lang="ts">
  // Uma coluna da arena: estado ao vivo (da thread), tempo, tokens/custo,
  // testes, tamanho do diff, votos, vencedor e o diff contra a base.
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { threadsStore } from "$lib/central/threads/threads-store.svelte";
  import { driverTint } from "$lib/central/threads-ui/drivers";
  import type { DiffResult } from "$lib/central/vcs";
  import {
    arenaDiff,
    arenaError,
    elapsedOf,
    formatCost,
    formatMs,
    formatTokens,
    isActive,
    type ArenaEntry,
    type ArenaView,
  } from "$lib/central/arena";
  import Icon from "$components/central/threads/Icon.svelte";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";

  let {
    arena,
    entry,
    now,
    tags = [],
    diffHeight = "60vh",
    showDiff = true,
    onvote,
    onpick,
    onverify,
    onapply,
  }: {
    arena: ArenaView;
    entry: ArenaEntry;
    now: number;
    tags?: string[];
    diffHeight?: string;
    showDiff?: boolean;
    onvote: (e: ArenaEntry, delta: 1 | -1) => void;
    onpick: (e: ArenaEntry) => void;
    onverify: (e: ArenaEntry) => void;
    onapply: (e: ArenaEntry) => void;
  } = $props();

  let thread = $derived(threadsStore.threads.find((x) => x.threadId === entry.threadId) ?? null);
  let inst = $derived(threadsStore.instances.find((i) => i.instance.id === entry.instanceId) ?? null);
  let live = $derived(thread?.status ?? null);
  let active = $derived(isActive(entry));
  let applied = $derived(arena.status === "applied");

  let diff = $state<DiffResult | null>(null);
  let diffError = $state<string | null>(null);
  let loading = $state(false);
  let testOpen = $state(false);
  let mode = $state<"unified" | "split">("unified");

  // Recarrega o diff quando o commit da entrada muda (fim de turno) ou ao abrir.
  let diffKey = $derived(`${entry.entryId}:${entry.commitSha ?? ""}:${entry.status === "done" || entry.status === "failed" || entry.status === "interrupted"}`);
  $effect(() => {
    void diffKey;
    if (showDiff) untrack(() => void loadDiff());
  });

  async function loadDiff() {
    if (entry.status === "preparing" && !entry.worktreePath) return;
    loading = true;
    diffError = null;
    try {
      diff = await arenaDiff(arena.arenaId, entry.entryId);
    } catch (e) {
      diffError = arenaError(e).message;
    } finally {
      loading = false;
    }
  }

  let statusLabel = $derived.by(() => {
    if (entry.status === "running" && (live === "approval" || live === "input")) return $t(`llm.central.arena.live.${live}`);
    return $t(`llm.central.arena.status.${entry.status}`);
  });
  let statusTone = $derived(
    entry.status === "done"
      ? "ok"
      : entry.status === "failed"
        ? "bad"
        : entry.status === "interrupted"
          ? "muted"
          : live === "approval" || live === "input"
            ? "warn"
            : "live",
  );
  let tokens = $derived(entry.inputTokens + entry.outputTokens);
</script>

<article class="col" class:winner={entry.winner} style:--tint={driverTint(entry.driver, inst)} aria-label={entry.label}>
  <header class="top">
    <span class="dot"></span>
    <div class="who">
      <strong title={entry.label}>{entry.label}</strong>
      <span class="sub">{entry.model ?? entry.instanceId}{#if entry.branch} · <span class="mono">{entry.branch}</span>{/if}</span>
    </div>
    {#if entry.winner}
      <span class="crown" title={$t("llm.central.arena.winner")}><Icon name="check-circle" size={16} /></span>
    {/if}
  </header>

  <div class="status tone-{statusTone}">
    {#if active}<span class="spin"><Icon name="circle-notch" size={13} /></span>{/if}
    <span>{statusLabel}</span>
    {#if entry.status === "running" && (live === "approval" || live === "input")}
      <a class="link" href={`/llm/threads?thread=${encodeURIComponent(entry.threadId)}`}>{$t("llm.central.arena.answer")}</a>
    {/if}
    <div class="grow"></div>
    {#each tags as tag (tag)}
      <span class="tag">{$t(`llm.central.arena.tag.${tag}`)}</span>
    {/each}
  </div>

  <dl class="stats">
    <div><dt><Icon name="clock" size={12} /> {$t("llm.central.arena.col.time")}</dt><dd>{formatMs(elapsedOf(entry, now))}</dd></div>
    <div><dt><Icon name="gauge" size={12} /> {$t("llm.central.arena.col.tokens")}</dt><dd title={`in ${entry.inputTokens} · out ${entry.outputTokens} · cached ${entry.cachedTokens}`}>{active && !tokens ? "…" : formatTokens(tokens)}</dd></div>
    <div><dt><Icon name="currency-dollar" size={12} /> {$t("llm.central.arena.col.cost")}</dt><dd>{formatCost(entry.costUsd)}</dd></div>
    <div>
      <dt><Icon name="git-diff" size={12} /> {$t("llm.central.arena.col.diff")}</dt>
      <dd>
        {#if entry.commitSha || diff}
          {@const files = diff?.files.length ?? entry.filesChanged}
          <span class="add">+{diff?.additions ?? entry.additions}</span>
          <span class="del">−{diff?.deletions ?? entry.deletions}</span>
          <span class="muted">· {$t("llm.central.arena.files", { count: files })}</span>
        {:else}—{/if}
      </dd>
    </div>
    <div class="wide">
      <dt><Icon name="list-checks" size={12} /> {$t("llm.central.arena.col.tests")}</dt>
      <dd>
        {#if entry.testStatus === "none"}
          <span class="muted">{arena.verifyCommand ? $t("llm.central.arena.test.pending") : $t("llm.central.arena.test.none")}</span>
        {:else}
          <button type="button" class="test test-{entry.testStatus}" onclick={() => (testOpen = !testOpen)} aria-expanded={testOpen}>
            {#if entry.testStatus === "running"}<Icon name="circle-notch" size={12} />{:else if entry.testStatus === "passed"}<Icon name="check-circle" size={12} />{:else}<Icon name="x-circle" size={12} />{/if}
            {$t(`llm.central.arena.test.${entry.testStatus}`)}
            {#if entry.testExit != null && entry.testStatus !== "passed"}<span class="mono">({entry.testExit})</span>{/if}
            {#if entry.testMs != null}<span class="muted">{formatMs(entry.testMs)}</span>{/if}
          </button>
        {/if}
        {#if arena.verifyCommand && !active && entry.worktreePath && !entry.cleanedAt}
          <button type="button" class="mini" onclick={() => onverify(entry)} title={$t("llm.central.arena.test.rerun")}>
            <Icon name="arrow-counter-clockwise" size={12} />
          </button>
        {/if}
      </dd>
    </div>
  </dl>
  {#if testOpen && entry.testTail}
    <pre class="tail">{entry.testTail}</pre>
  {/if}
  {#if entry.error}
    <p class="error" title={entry.error}>{entry.error}</p>
  {/if}

  <div class="actions">
    <div class="votes" role="group" aria-label={$t("llm.central.arena.votes")}>
      <button type="button" class="mini" onclick={() => onvote(entry, 1)} disabled={applied} aria-label={$t("llm.central.arena.vote_up")}><Icon name="arrow-up" size={12} /></button>
      <span class="count">{entry.votes}</span>
      <button type="button" class="mini" onclick={() => onvote(entry, -1)} disabled={applied || entry.votes === 0} aria-label={$t("llm.central.arena.vote_down")}><Icon name="arrow-down" size={12} /></button>
    </div>
    <button type="button" class="pill" class:on={entry.winner} onclick={() => onpick(entry)} disabled={applied || entry.status === "failed"}>
      <Icon name="check" size={12} /> {entry.winner ? $t("llm.central.arena.picked") : $t("llm.central.arena.pick")}
    </button>
    <div class="grow"></div>
    <a class="mini" href={`/llm/threads?thread=${encodeURIComponent(entry.threadId)}`} title={$t("llm.central.arena.open_thread")}><Icon name="arrow-square-out" size={13} /></a>
    {#if entry.winner && !applied}
      <button type="button" class="pill primary" onclick={() => onapply(entry)}><Icon name="git-branch" size={12} /> {$t("llm.central.arena.apply_winner")}</button>
    {/if}
  </div>

  {#if showDiff}
    <div class="diff">
      <div class="diff-bar">
        <span class="muted">{$t("llm.central.arena.diff_vs", { base: arena.baseBranch ?? arena.baseCommit.slice(0, 7) })}</span>
        <div class="grow"></div>
        <button type="button" class="mini" onclick={() => (mode = mode === "unified" ? "split" : "unified")} title={$t("llm.central.arena.diff_mode")}>
          <Icon name="arrows-split" size={12} />
        </button>
        <button type="button" class="mini" onclick={loadDiff} disabled={loading} title={$t("llm.central.arena.refresh")}>
          <Icon name={loading ? "circle-notch" : "arrow-counter-clockwise"} size={12} />
        </button>
      </div>
      {#if diffError}
        <p class="error">{diffError}</p>
      {:else if diff}
        <DiffViewer files={diff.files} bind:mode height={diffHeight} toolbar={false} collapsedByDefault={diff.files.length > 6} emptyText={$t("llm.central.arena.no_changes") as string} />
      {:else}
        <p class="muted pad">{active ? $t("llm.central.arena.working") : "—"}</p>
      {/if}
    </div>
  {/if}
</article>

<style>
  .col { min-width: 0; display: flex; flex-direction: column; gap: 8px; border: 1px solid var(--separator); border-top: 3px solid var(--tint); border-radius: var(--radius-lg); padding: 10px; background: var(--surface); }
  .col.winner { border-color: color-mix(in srgb, var(--success) 55%, var(--separator)); box-shadow: 0 0 0 1px color-mix(in srgb, var(--success) 40%, transparent); }
  .top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--tint); flex-shrink: 0; }
  .who { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .who strong { font-size: var(--text-md); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--text-xs); color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .crown { color: var(--success); }
  .mono { font-family: var(--font-mono); }
  .status { display: flex; align-items: center; gap: 6px; font-size: var(--text-sm); font-weight: 600; padding: 4px 8px; border-radius: var(--radius-md); background: var(--fill-1); }
  .tone-ok { color: var(--success); }
  .tone-bad { color: var(--error); }
  .tone-warn { color: var(--warning); background: color-mix(in srgb, var(--warning) 12%, transparent); }
  .tone-live { color: var(--accent-text, var(--accent)); }
  .tone-muted { color: var(--text-muted); }
  .spin { display: inline-flex; animation: spin 900ms linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .link { color: inherit; text-decoration: underline; font-weight: 500; }
  .tag { font-size: var(--text-caption); font-weight: 600; text-transform: uppercase; letter-spacing: var(--track-caps); padding: 1px 6px; border-radius: 999px; background: color-mix(in srgb, var(--tint) 16%, transparent); color: var(--text); }
  .stats { display: grid; grid-template-columns: 1fr 1fr; gap: 6px 10px; margin: 0; }
  .stats > div { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .stats .wide { grid-column: 1 / -1; }
  dt { font-size: var(--text-xs); color: var(--text-muted); display: inline-flex; align-items: center; gap: 4px; }
  dd { margin: 0; font-size: var(--text-base); font-variant-numeric: tabular-nums; display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .add { color: var(--success); }
  .del { color: var(--error); }
  .muted { color: var(--text-muted); font-size: var(--text-sm); }
  .pad { padding: 12px; margin: 0; }
  .test { display: inline-flex; align-items: center; gap: 4px; border: 0; background: var(--fill-1); color: var(--text); font-size: var(--text-sm); padding: 2px 8px; border-radius: 999px; cursor: pointer; }
  .test-passed { color: var(--success); }
  .test-failed, .test-error { color: var(--error); }
  .tail { margin: 0; max-height: 200px; overflow: auto; font-family: var(--font-mono); font-size: var(--text-xs); white-space: pre-wrap; background: var(--fill-1); border-radius: var(--radius-md); padding: 8px; }
  .error { margin: 0; color: var(--error); font-size: var(--text-sm); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .actions { display: flex; align-items: center; gap: 6px; }
  .votes { display: inline-flex; align-items: center; gap: 2px; }
  .count { min-width: 18px; text-align: center; font-variant-numeric: tabular-nums; font-size: var(--text-sm); }
  .grow { flex: 1; }
  .mini { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); cursor: pointer; text-decoration: none; }
  .mini:hover:not(:disabled) { background: var(--fill-2); color: var(--text); }
  .mini:disabled { opacity: 0.4; cursor: default; }
  .pill { display: inline-flex; align-items: center; gap: 4px; border: 1px solid var(--separator); background: transparent; color: var(--text); font-size: var(--text-sm); padding: 3px 9px; border-radius: 999px; cursor: pointer; }
  .pill:hover:not(:disabled) { background: var(--fill-1); }
  .pill.on { border-color: var(--success); color: var(--success); background: color-mix(in srgb, var(--success) 10%, transparent); }
  .pill.primary { background: var(--cta); color: var(--on-cta); border-color: transparent; }
  .pill:disabled { opacity: 0.5; cursor: default; }
  .diff { display: flex; flex-direction: column; min-height: 0; border-top: 1px solid var(--separator); padding-top: 6px; gap: 4px; }
  .diff-bar { display: flex; align-items: center; gap: 4px; }
</style>
