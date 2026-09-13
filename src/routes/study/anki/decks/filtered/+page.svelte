<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type DeckSummary = {
    id: number;
    name: string;
    filtered: boolean;
    new_count: number;
    learn_count: number;
    review_count: number;
  };

  type FilteredSearchTerm = {
    search: string;
    limit: number;
    order: string;
  };

  let decks = $state<DeckSummary[]>([]);
  let loading = $state(true);
  let error = $state("");
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let createOpen = $state(false);
  let newName = $state("");
  let newSearch = $state("is:due");
  let newLimit = $state(100);
  let newOrder = $state("oldest_seen_first");
  let newReschedule = $state(true);
  let createBusy = $state(false);

  let rebuilding = $state<number | null>(null);
  let emptying = $state<number | null>(null);

  let confirmDeleteOpen = $state(false);
  let deleteTarget = $state<DeckSummary | null>(null);
  let deleteBusy = $state(false);

  const ORDER_OPTIONS = $derived([
    { value: "oldest_seen_first", label: $t("study.anki.filtered.ord_oldest_seen") },
    { value: "random", label: $t("study.anki.filtered.ord_random") },
    { value: "interval_descending", label: $t("study.anki.filtered.ord_interval_desc") },
    { value: "interval_ascending", label: $t("study.anki.filtered.ord_interval_asc") },
    { value: "lapses_descending", label: $t("study.anki.filtered.ord_lapses_desc") },
    { value: "added_descending", label: $t("study.anki.filtered.ord_added_desc") },
    { value: "added_ascending", label: $t("study.anki.filtered.ord_added_asc") },
    { value: "due_first", label: $t("study.anki.filtered.ord_due_first") },
  ]);

  const PRESETS = $derived([
    { label: $t("study.anki.filtered.p_due_today"), search: "is:due" },
    { label: $t("study.anki.browse.p_learning"), search: "is:learn" },
    { label: $t("study.anki.browse.p_suspended"), search: "is:suspended" },
    { label: $t("study.anki.browse.p_flagged"), search: "flag:1 OR flag:2 OR flag:3 OR flag:4" },
    { label: $t("study.anki.filtered.p_tagged_hard"), search: "tag:difícil" },
    { label: $t("study.anki.filtered.p_lapses_5"), search: "prop:lapses>5" },
  ]);

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const all = await pluginInvoke<DeckSummary[]>(
        "study",
        "study:anki:decks:list",
      );
      decks = all.filter((d) => d.filtered);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function createFiltered() {
    const name = newName.trim();
    if (!name || !newSearch.trim()) return;
    createBusy = true;
    try {
      const term: FilteredSearchTerm = {
        search: newSearch.trim(),
        limit: Math.max(1, Math.min(99999, Math.floor(newLimit))),
        order: newOrder,
      };
      await pluginInvoke<{ id: number; cards: number }>(
        "study",
        "study:anki:decks:create_filtered",
        {
          name,
          searchTerms: [term],
          reschedule: newReschedule,
        },
      );
      showToast("ok", `Deck "${name}" criado`);
      createOpen = false;
      newName = "";
      newSearch = "is:due";
      newLimit = 100;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      createBusy = false;
    }
  }

  async function rebuild(deck: DeckSummary) {
    rebuilding = deck.id;
    try {
      const r = await pluginInvoke<{ cards: number }>(
        "study",
        "study:anki:decks:rebuild_filtered",
        { id: deck.id },
      );
      showToast(
        "ok",
        r.cards === 1
          ? "1 card no deck filtrado"
          : `${r.cards} cards no deck filtrado`,
      );
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      rebuilding = null;
    }
  }

  async function emptyDeck(deck: DeckSummary) {
    emptying = deck.id;
    try {
      const r = await pluginInvoke<{ returned: number }>(
        "study",
        "study:anki:decks:empty_filtered",
        { id: deck.id },
      );
      showToast(
        "ok",
        r.returned === 0
          ? $t("study.anki.filtered.was_empty")
          : r.returned === 1
            ? $t("study.anki.filtered.returned_one")
            : $t("study.anki.filtered.returned_many", { n: r.returned }),
      );
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      emptying = null;
    }
  }

  function askDelete(deck: DeckSummary) {
    deleteTarget = deck;
    confirmDeleteOpen = true;
  }

  async function confirmDelete() {
    if (!deleteTarget) return;
    deleteBusy = true;
    try {
      await pluginInvoke("study", "study:anki:decks:delete_filtered", {
        id: deleteTarget.id,
      });
      showToast("ok", "Deck filtrado removido");
      confirmDeleteOpen = false;
      deleteTarget = null;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      deleteBusy = false;
    }
  }

  onMount(load);
</script>

<section class="study-page">
  <PageHero
    title="Decks filtrados"
    subtitle={$t("study.anki.filtered.subtitle")}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="toolbar">
    <a class="back-link" href="/study/anki/decks">← {$t("study.anki.sidebar.decks")}</a>
    <button
      type="button"
      class="btn primary"
      onclick={() => (createOpen = true)}
    >
      + {$t("study.anki.filtered.new_filtered_deck")}
    </button>
  </div>

  {#if loading}
    <div class="state">{$t("study.anki.settings.loading")}</div>
  {:else if error}
    <div class="state err">{error}</div>
    <button class="btn ghost" onclick={load}>{$t("study.anki.settings.try_again")}</button>
  {:else if decks.length === 0}
    <div class="empty">
      <p>{$t("study.anki.filtered.none_yet")}</p>
      <p class="hint">
        {$t("study.anki.filtered.explain_a")} <code>is:due</code>. {$t("study.anki.filtered.explain_b")}
      </p>
      <button
        type="button"
        class="btn primary"
        onclick={() => (createOpen = true)}
      >
        {$t("study.anki.notetypes.create_first")}
      </button>
    </div>
  {:else}
    <ul class="filtered-list">
      {#each decks as deck (deck.id)}
        {@const total = deck.new_count + deck.learn_count + deck.review_count}
        <li class="filtered-row">
          <div class="filtered-info">
            <h3>{deck.name}</h3>
            <p class="meta">
              <span class="pill new">{$t("study.anki.filtered.n_new", { n: deck.new_count })}</span>
              <span class="pill learn">{$t("study.anki.filtered.n_learning", { n: deck.learn_count })}</span>
              <span class="pill review">{$t("study.anki.filtered.n_review", { n: deck.review_count })}</span>
            </p>
          </div>
          <div class="filtered-actions">
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => rebuild(deck)}
              disabled={rebuilding === deck.id || emptying === deck.id}
              title={$t("study.anki.filtered.rebuild_hint")}
            >
              {rebuilding === deck.id ? $t("study.anki.filtered.rebuilding") : $t("study.anki.filtered.rebuild")}
            </button>
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => emptyDeck(deck)}
              disabled={rebuilding === deck.id || emptying === deck.id || total === 0}
              title={$t("study.anki.filtered.empty_hint")}
            >
              {emptying === deck.id ? $t("study.anki.media.emptying") : $t("study.anki.media.empty")}
            </button>
            <button
              type="button"
              class="btn ghost sm danger"
              onclick={() => askDelete(deck)}
              disabled={rebuilding === deck.id || emptying === deck.id}
            >
              {$t("study.common.delete")}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if createOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) createOpen = false; }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>{$t("study.anki.filtered.create_title")}</h3>
      <p class="modal-hint">
        {$t("study.anki.filtered.create_hint")} <code>is:due</code>, <code>tag:foo</code>, <code>deck:Bar</code>.
      </p>

      <label class="field">
        <span>{$t("study.anki.notetypes.name_label")}</span>
        <input
          type="text"
          bind:value={newName}
          placeholder={$t("study.anki.filtered.name_placeholder")}
        />
      </label>

      <label class="field">
        <span>Query</span>
        <input
          type="text"
          bind:value={newSearch}
          placeholder="is:due"
        />
      </label>

      <details class="presets">
        <summary>{$t("study.anki.decks.presets")}</summary>
        <div class="preset-grid">
          {#each PRESETS as p (p.label)}
            <button
              type="button"
              class="preset-btn"
              onclick={() => (newSearch = p.search)}
            >
              <span class="preset-label">{p.label}</span>
              <span class="preset-q">{p.search}</span>
            </button>
          {/each}
        </div>
      </details>

      <div class="row">
        <label class="field">
          <span>{$t("study.anki.filtered.limit")}</span>
          <input
            type="number"
            min="1"
            max="9999"
            bind:value={newLimit}
          />
        </label>

        <label class="field">
          <span>Ordem</span>
          <select bind:value={newOrder}>
            {#each ORDER_OPTIONS as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </label>
      </div>

      <label class="check">
        <input type="checkbox" bind:checked={newReschedule} />
        <span>Re-agendar cards (Anki recomenda manter ligado)</span>
      </label>

      <div class="modal-foot">
        <button
          type="button"
          class="btn ghost"
          onclick={() => (createOpen = false)}
          disabled={createBusy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={createFiltered}
          disabled={createBusy || !newName.trim() || !newSearch.trim()}
        >
          {createBusy ? "Criando…" : "Criar"}
        </button>
      </div>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmDeleteOpen}
  title={$t("study.anki.filtered.delete_title")}
  message={deleteTarget
    ? $t("study.anki.filtered.delete_confirm", { name: deleteTarget.name })
    : ""}
  confirmLabel={$t("study.common.delete")}
  variant="danger"
  onConfirm={confirmDelete}
/>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 820px;
    margin-inline: auto;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .back-link {
    color: var(--accent);
    font-size: 13px;
    text-decoration: none;
  }
  .back-link:hover {
    text-decoration: underline;
  }

  .state {
    padding: 24px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }

  .empty {
    padding: 48px 24px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  .empty .hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    max-width: 420px;
    line-height: 1.55;
  }

  .filtered-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .filtered-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
  }
  .filtered-info {
    flex: 1;
    min-width: 0;
  }
  .filtered-info h3 {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 600;
  }
  .meta {
    display: flex;
    gap: 6px;
    margin: 0;
  }
  .pill {
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .pill.new {
    background: color-mix(in oklab, var(--blue, var(--accent)) 14%, transparent);
    color: var(--blue, var(--accent));
  }
  .pill.learn {
    background: color-mix(in oklab, var(--orange, var(--accent)) 14%, transparent);
    color: var(--orange, var(--accent));
  }
  .pill.review {
    background: color-mix(in oklab, var(--green, var(--accent)) 14%, transparent);
    color: var(--green, var(--accent));
  }

  .filtered-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .btn {
    padding: 7px 14px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover, var(--accent));
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.ghost.danger {
    color: var(--error, var(--accent));
    border-color: color-mix(in oklab, var(--error, var(--accent)) 35%, var(--input-border));
  }
  .btn.ghost.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error, var(--accent)) 10%, transparent);
  }
  .btn.sm {
    padding: 5px 10px;
    font-size: 12px;
  }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 20%, transparent);
    z-index: 100;
  }
  .toast.err {
    background: color-mix(in oklab, var(--error, var(--accent)) 18%, var(--surface));
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.55));
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    padding: 16px;
  }
  .modal {
    background: var(--popup-bg, var(--surface));
    border: none;
    border-radius: var(--border-radius);
    padding: 20px;
    max-width: 540px;
    width: 100%;
    box-shadow: 0 20px 50px color-mix(in oklab, black 30%, transparent);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modal h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .modal-hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    line-height: 1.55;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .field input,
  .field select {
    padding: 8px 10px;
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
  .row {
    display: grid;
    grid-template-columns: 1fr 2fr;
    gap: 8px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--secondary);
    font-size: 12px;
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
  }

  .presets summary {
    cursor: pointer;
    color: var(--accent);
    font-size: 12px;
  }
  .preset-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    margin-top: 8px;
  }
  .preset-btn {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 10px;
    background: var(--bg);
    border: none;
    border-radius: var(--border-radius);
    text-align: left;
    cursor: pointer;
  }
  .preset-btn:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
    border-color: var(--accent);
  }
  .preset-label {
    font-size: 12px;
    color: var(--text);
  }
  .preset-q {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
    color: var(--tertiary);
  }

  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    border-top: none;
    padding-top: 12px;
  }

  code {
    padding: 1px 6px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 4px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
</style>
