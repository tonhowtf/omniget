<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type RevlogEntry = {
    id: number;
    cid: number;
    ease: number;
    ivl: number;
    last_ivl: number;
    factor: number;
    time_ms: number;
    kind: string;
  };

  type DeckSummary = { id: number; name: string };
  type NotetypeSummary = { id: number; name: string };
  type TagSummary = { tag: string };

  type Mode = "card" | "deck" | "notetype" | "tag" | "range";

  let mode = $state<Mode>("range");
  let entries = $state<RevlogEntry[]>([]);
  let loading = $state(false);
  let error = $state("");

  let cardId = $state<number | "">("");
  let deckId = $state<number | null>(null);
  let notetypeId = $state<number | null>(null);
  let tag = $state("");
  let rangeDays = $state(7);

  let decks = $state<DeckSummary[]>([]);
  let notetypes = $state<NotetypeSummary[]>([]);
  let tags = $state<TagSummary[]>([]);

  let limit = $state(200);
  let offset = $state(0);

  async function loadLookups() {
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const [d, n, t] = await Promise.all([
        pluginInvoke<DeckSummary[]>("study", "study:anki:decks:list"),
        pluginInvoke<NotetypeSummary[]>("study", "study:anki:notetypes:list"),
        pluginInvoke<TagSummary[]>("study", "study:anki:tags:list"),
      ]);
      decks = d ?? [];
      notetypes = n ?? [];
      tags = t ?? [];
      if (deckId == null && decks.length > 0) deckId = decks[0].id;
      if (notetypeId == null && notetypes.length > 0) notetypeId = notetypes[0].id;
    } catch (e) {
      console.error(e);
    }
  }

  async function run() {
    error = "";
    loading = true;
    try {
      let res: RevlogEntry[] = [];
      if (mode === "card") {
        if (cardId === "" || !Number.isFinite(Number(cardId))) {
          error = $t("study.anki.revlog.err_cardid");
          entries = [];
          return;
        }
        res = await pluginInvoke<RevlogEntry[]>(
          "study",
          "study:anki:revlog:list_by_card",
          { cardId: Number(cardId) },
        );
      } else if (mode === "deck") {
        if (deckId == null) return;
        res = await pluginInvoke<RevlogEntry[]>(
          "study",
          "study:anki:revlog:list_by_deck",
          { deckId, limit, offset },
        );
      } else if (mode === "notetype") {
        if (notetypeId == null) return;
        res = await pluginInvoke<RevlogEntry[]>(
          "study",
          "study:anki:revlog:list_by_notetype",
          { notetypeId, limit, offset },
        );
      } else if (mode === "tag") {
        if (!tag.trim()) {
          error = $t("study.anki.revlog.err_tag");
          entries = [];
          return;
        }
        res = await pluginInvoke<RevlogEntry[]>(
          "study",
          "study:anki:revlog:list_by_tag",
          { tag: tag.trim(), limit, offset },
        );
      } else {
        const endSecs = Math.floor(Date.now() / 1000);
        const startSecs = endSecs - Math.max(1, Math.floor(rangeDays)) * 86400;
        res = await pluginInvoke<RevlogEntry[]>(
          "study",
          "study:anki:revlog:list_in_range",
          { startSecs, endSecs, limit, offset },
        );
      }
      entries = res ?? [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      entries = [];
    } finally {
      loading = false;
    }
  }

  function fmtDate(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  function fmtIvl(days: number): string {
    if (days === 0) return "—";
    if (days < 30) return `${days}d`;
    if (days < 365) return `${Math.round(days / 30)}m`;
    return `${(days / 365).toFixed(1)}a`;
  }

  function easeLabel(ease: number): string {
    return [$t("study.anki.revlog.ease_q"), $t("study.anki.revlog.ease_again"), $t("study.anki.revlog.ease_hard"), $t("study.anki.revlog.ease_good"), $t("study.anki.revlog.ease_easy")][ease] ?? `?${ease}`;
  }

  function easeClass(ease: number): string {
    return ["", "ease-1", "ease-2", "ease-3", "ease-4"][ease] ?? "";
  }

  let summary = $derived.by(() => {
    if (entries.length === 0) return null;
    const counts = [0, 0, 0, 0, 0];
    let totalMs = 0;
    for (const e of entries) {
      if (e.ease >= 0 && e.ease <= 4) counts[e.ease]++;
      totalMs += e.time_ms;
    }
    return {
      total: entries.length,
      counts,
      avgSec: (totalMs / entries.length / 1000).toFixed(1),
    };
  });

  onMount(async () => {
    await loadLookups();
    await run();
  });
</script>

<section class="study-page">
  <PageHero
    title="Revlog"
    subtitle={$t("study.anki.revlog.subtitle")}
  />

  <div class="toolbar">
    <a href="/study/anki/stats" class="back-link">← {$t("study.anki.sidebar.stats")}</a>
  </div>

  <div class="filter-card">
    <div class="mode-tabs" role="tablist" aria-label={$t("study.anki.stats.period_aria")}>
      <button
        type="button"
        class="mode-tab"
        class:active={mode === "range"}
        role="tab"
        onclick={() => (mode = "range")}
      >{$t("study.anki.stats.period_aria")}</button>
      <button
        type="button"
        class="mode-tab"
        class:active={mode === "deck"}
        role="tab"
        onclick={() => (mode = "deck")}
      >Deck</button>
      <button
        type="button"
        class="mode-tab"
        class:active={mode === "notetype"}
        role="tab"
        onclick={() => (mode = "notetype")}
      >{$t("study.anki.notetypes.title")}</button>
      <button
        type="button"
        class="mode-tab"
        class:active={mode === "tag"}
        role="tab"
        onclick={() => (mode = "tag")}
      >Tag</button>
      <button
        type="button"
        class="mode-tab"
        class:active={mode === "card"}
        role="tab"
        onclick={() => (mode = "card")}
      >{$t("study.anki.revlog.specific_card")}</button>
    </div>

    <div class="filter-body">
      {#if mode === "range"}
        <label class="field">
          <span>{$t("study.anki.revlog.last_n_days")}</span>
          <input
            type="number"
            min="1"
            max="3650"
            bind:value={rangeDays}
          />
        </label>
      {:else if mode === "deck"}
        <label class="field">
          <span>Deck</span>
          <select bind:value={deckId}>
            {#each decks as d (d.id)}
              <option value={d.id}>{d.name}</option>
            {/each}
          </select>
        </label>
      {:else if mode === "notetype"}
        <label class="field">
          <span>{$t("study.anki.revlog.th_notetype")}</span>
          <select bind:value={notetypeId}>
            {#each notetypes as n (n.id)}
              <option value={n.id}>{n.name}</option>
            {/each}
          </select>
        </label>
      {:else if mode === "tag"}
        <label class="field">
          <span>Tag</span>
          <input
            type="text"
            bind:value={tag}
            placeholder="exemplo::tag"
            list="tag-list"
          />
          <datalist id="tag-list">
            {#each tags as t (t.tag)}
              <option value={t.tag}></option>
            {/each}
          </datalist>
        </label>
      {:else}
        <label class="field">
          <span>Card ID</span>
          <input
            type="number"
            bind:value={cardId}
            placeholder="123456789"
          />
        </label>
      {/if}

      {#if mode !== "card"}
        <label class="field">
          <span>Limite</span>
          <input type="number" min="1" max="5000" bind:value={limit} />
        </label>
      {/if}

      <button
        type="button"
        class="btn primary"
        onclick={run}
        disabled={loading}
      >
        {loading ? $t("study.anki.revlog.searching") : $t("study.anki.sidebar.browse")}
      </button>
    </div>
  </div>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  {#if summary}
    <div class="summary-row">
      <div class="sum-stat">
        <span class="sum-num">{summary.total}</span>
        <span class="sum-label">{summary.total === 1 ? "review" : "reviews"}</span>
      </div>
      <div class="sum-stat ease-1">
        <span class="sum-num">{summary.counts[1]}</span>
        <span class="sum-label">errei</span>
      </div>
      <div class="sum-stat ease-2">
        <span class="sum-num">{summary.counts[2]}</span>
        <span class="sum-label">{$t("study.anki.revlog.ease_hard_l")}</span>
      </div>
      <div class="sum-stat ease-3">
        <span class="sum-num">{summary.counts[3]}</span>
        <span class="sum-label">bom</span>
      </div>
      <div class="sum-stat ease-4">
        <span class="sum-num">{summary.counts[4]}</span>
        <span class="sum-label">{$t("study.anki.revlog.ease_easy_l")}</span>
      </div>
      <div class="sum-stat">
        <span class="sum-num">{summary.avgSec}s</span>
        <span class="sum-label">{$t("study.anki.revlog.ease_good_l")}</span>
      </div>
    </div>
  {/if}

  {#if loading}
    <p class="muted">{$t("study.anki.settings.loading")}</p>
  {:else if entries.length === 0 && !error}
    <p class="muted center">{$t("study.anki.revlog.no_entries")}</p>
  {:else if entries.length > 0}
    <div class="table-wrap">
      <table class="revlog-table">
        <thead>
          <tr>
            <th class="when">Quando</th>
            <th class="card-col">Card</th>
            <th class="ease-col">Resp.</th>
            <th class="num">Ivl</th>
            <th class="num">→ Ivl</th>
            <th class="num">Tempo</th>
            <th class="kind-col">Tipo</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as e (e.id)}
            <tr>
              <td class="mono">{fmtDate(e.id)}</td>
              <td class="mono">#{e.cid}</td>
              <td>
                <span class="ease {easeClass(e.ease)}">
                  {easeLabel(e.ease)}
                </span>
              </td>
              <td class="num mono">{fmtIvl(e.last_ivl)}</td>
              <td class="num mono">{fmtIvl(e.ivl)}</td>
              <td class="num mono">{(e.time_ms / 1000).toFixed(1)}s</td>
              <td class="kind">{e.kind}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 980px;
    margin-inline: auto;
  }

  .toolbar { display: flex; }
  .back-link {
    color: var(--accent);
    font-size: 13px;
    text-decoration: none;
  }
  .back-link:hover { text-decoration: underline; }

  .filter-card {
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 4px;
  }
  .mode-tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    overflow-x: auto;
  }
  .mode-tab {
    padding: 6px 12px;
    border: 0;
    background: transparent;
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    white-space: nowrap;
    cursor: pointer;
  }
  .mode-tab.active {
    background: color-mix(in oklab, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .mode-tab:hover:not(.active) {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }

  .filter-body {
    display: flex;
    align-items: flex-end;
    gap: 12px;
    padding: 12px 16px;
    flex-wrap: wrap;
    border-top: none;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
    flex: 1;
    min-width: 140px;
  }
  .field input,
  .field select {
    padding: 7px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .field input:focus,
  .field select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .btn {
    padding: 7px 16px;
    border-radius: var(--border-radius);
    border: 0;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.08); }

  .summary-row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .sum-stat {
    flex: 1;
    min-width: 90px;
    padding: 10px 14px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sum-num {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .sum-label {
    font-size: 11px;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .sum-stat.ease-1 .sum-num { color: var(--error, var(--accent)); }
  .sum-stat.ease-2 .sum-num { color: var(--orange, var(--accent)); }
  .sum-stat.ease-3 .sum-num { color: var(--green, var(--accent)); }
  .sum-stat.ease-4 .sum-num { color: var(--accent); }

  .error {
    color: var(--error, var(--accent));
    font-size: 13px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--error, var(--accent)) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--error, var(--accent)) 25%, var(--input-border));
    border-radius: var(--border-radius);
  }

  .muted { color: var(--tertiary); font-size: 13px; }
  .center { text-align: center; padding: 24px; }

  .table-wrap {
    overflow-x: auto;
    border: none;
    border-radius: var(--border-radius);
  }
  .revlog-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .revlog-table th {
    text-align: left;
    padding: 8px 10px;
    background: var(--surface);
    color: var(--tertiary);
    font-weight: 500;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    border-bottom: none;
  }
  .revlog-table td {
    padding: 6px 10px;
    border-bottom: none;
    color: var(--text);
  }
  .revlog-table tr:last-child td {
    border-bottom: 0;
  }
  .revlog-table .num { text-align: right; }
  .revlog-table .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
  .revlog-table .when { white-space: nowrap; }

  .ease {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
  }
  .ease.ease-1 {
    background: color-mix(in oklab, var(--error, var(--accent)) 14%, transparent);
    color: var(--error, var(--accent));
  }
  .ease.ease-2 {
    background: color-mix(in oklab, var(--orange, var(--accent)) 14%, transparent);
    color: var(--orange, var(--accent));
  }
  .ease.ease-3 {
    background: color-mix(in oklab, var(--green, var(--accent)) 14%, transparent);
    color: var(--green, var(--accent));
  }
  .ease.ease-4 {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .kind {
    color: var(--tertiary);
    font-size: 11px;
  }
</style>
