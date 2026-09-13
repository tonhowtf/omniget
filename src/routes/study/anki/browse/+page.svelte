<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type Card = {
    id: number;
    nid: number;
    did: number;
    ord: number;
    queue: string;
    ctype: string;
    due: number;
    ivl: number;
    factor: number;
    reps: number;
    lapses: number;
    flag: string;
    mod_secs: number;
  };

  type SearchResponse = { items: Card[]; total: number };

  type DeckSummary = {
    id: number;
    name: string;
    filtered: boolean;
  };

  type Note = {
    id: number;
    fields: string[];
    tags: string[];
  };

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

  type CardStats = {
    card_id: number;
    note_id: number;
    deck_id: number;
    first_review_ms: number | null;
    latest_review_ms: number | null;
    interval_days: number;
    ease_factor: number;
    reviews_count: number;
    lapses_count: number;
    avg_seconds: number;
    total_seconds: number;
    fsrs_memory_state: { stability: number; difficulty: number } | null;
    fsrs_retrievability: number | null;
    revlog: RevlogEntry[];
  };

  let query = $state("");
  let appliedQuery = $state("");
  let limit = $state(50);
  let offset = $state(0);

  let loading = $state(true);
  let error = $state("");
  let total = $state(0);
  let items = $state<Card[]>([]);
  let notesById = $state<Record<number, Note>>({});
  let decksById = $state<Record<number, string>>({});

  let selected = $state(new Set<number>());

  let drawerOpen = $state(false);
  let drawerCardId = $state<number | null>(null);
  let drawerStats = $state<CardStats | null>(null);
  let drawerNote = $state<Note | null>(null);
  let drawerError = $state("");

  let confirmOpen = $state(false);
  let confirmAction = $state<"delete" | null>(null);

  let bulkDeckPickerOpen = $state(false);
  let bulkDeckTarget = $state<number | null>(null);
  let allDecks = $state<DeckSummary[]>([]);

  let bulkTagOpen = $state(false);
  let bulkTagMode = $state<"add" | "remove">("add");
  let bulkTagInput = $state("");

  let unburyDeckPickerOpen = $state(false);
  let unburyDeckTarget = $state<number | null>(null);

  let editNoteOpen = $state(false);
  let editNoteTarget = $state<Note | null>(null);
  let editNoteFields = $state<string[]>([]);
  let editNoteTags = $state("");
  let editNoteBusy = $state(false);

  let confirmDeleteNoteOpen = $state(false);
  let deleteNoteTarget = $state<number | null>(null);
  let deleteNoteBusy = $state(false);

  let siblingCards = $state<Card[]>([]);
  let siblingsLoading = $state(false);

  let busy = $state(false);

  let searchTimer: number | null = null;

  const presetQueries = $derived([
    { label: $t("study.anki.browse.p_due"), q: "is:due" },
    { label: $t("study.anki.browse.p_new"), q: "is:new" },
    { label: $t("study.anki.browse.p_learning"), q: "is:learning" },
    { label: $t("study.anki.browse.p_suspended"), q: "is:suspended" },
    { label: $t("study.anki.browse.p_flagged"), q: "-flag:0" },
    { label: $t("study.anki.browse.p_lapsed_today"), q: "rated:1:1" },
  ]);

  function fmtDate(ms: number | null): string {
    if (!ms) return "—";
    const d = new Date(ms);
    return d.toLocaleDateString("pt-BR", {
      day: "2-digit",
      month: "short",
      year: "2-digit",
    });
  }

  function fmtRelativeSecs(secs: number): string {
    const now = Math.floor(Date.now() / 1000);
    const diff = now - secs;
    if (diff < 60) return "agora";
    if (diff < 3600) return `${Math.floor(diff / 60)}min`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
    if (diff < 86400 * 7) return `${Math.floor(diff / 86400)}d`;
    return fmtDate(secs * 1000);
  }

  function strip(html: string): string {
    return html
      .replace(/<[^>]*>/g, " ")
      .replace(/\s+/g, " ")
      .trim();
  }

  function flagDot(flag: string): string {
    switch (flag) {
      case "red":
        return "var(--error)";
      case "orange":
        return "var(--warning, var(--accent))";
      case "green":
        return "var(--success, var(--accent))";
      case "blue":
        return "var(--accent)";
      case "pink":
        return "color-mix(in oklab, var(--error) 70%, var(--accent))";
      case "turquoise":
        return "color-mix(in oklab, var(--success, var(--accent)) 60%, var(--accent))";
      case "purple":
        return "color-mix(in oklab, var(--accent) 70%, var(--error) 30%)";
      default:
        return "transparent";
    }
  }

  function queueLabel(q: string): string {
    switch (q) {
      case "new":
        return $t("study.anki.browse.state_new");
      case "learning":
        return $t("study.anki.browse.state_learning");
      case "review":
        return $t("study.anki.browse.state_review");
      case "day_learn_relearn":
        return "Relearn";
      case "suspended":
        return $t("study.anki.browse.state_suspended");
      case "user_buried":
        return "Buried";
      case "sched_buried":
        return "Buried (sib)";
      default:
        return q;
    }
  }

  async function runSearch() {
    loading = true;
    error = "";
    selected = new Set();
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const res = await pluginInvoke<SearchResponse>(
        "study",
        "study:anki:search:cards",
        { query: appliedQuery, limit, offset },
      );
      items = res.items;
      total = res.total;
      const noteIds = Array.from(new Set(items.map((c) => c.nid)));
      const deckIds = Array.from(new Set(items.map((c) => c.did)));
      const noteEntries = await Promise.all(
        noteIds.map((id) =>
          pluginInvoke<Note>("study", "study:anki:notes:get", { id })
            .then((n) => [id, n] as const)
            .catch(() => [id, null] as const),
        ),
      );
      const newNotes: Record<number, Note> = {};
      for (const [id, n] of noteEntries) if (n) newNotes[id] = n;
      notesById = newNotes;

      const missingDecks = deckIds.filter((id) => !decksById[id]);
      if (missingDecks.length > 0) {
        const list = await pluginInvoke<DeckSummary[]>(
          "study",
          "study:anki:decks:list",
        );
        allDecks = list;
        const map: Record<number, string> = { ...decksById };
        for (const d of list) map[d.id] = d.name;
        decksById = map;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function applyQueryDebounced() {
    if (searchTimer != null) clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      offset = 0;
      appliedQuery = query;
      runSearch();
    }, 300);
  }

  function applyPreset(q: string) {
    query = q;
    offset = 0;
    appliedQuery = q;
    runSearch();
  }

  $effect(() => {
    void query;
    applyQueryDebounced();
  });

  onMount(async () => {
    try {
      const list = await pluginInvoke<DeckSummary[]>("study", "study:anki:decks:list");
      allDecks = list;
      const map: Record<number, string> = {};
      for (const d of list) map[d.id] = d.name;
      decksById = map;
      bulkDeckTarget = list.find((d) => !d.filtered)?.id ?? null;
    } catch (e) {
      console.error(e);
    }
    appliedQuery = query;
    runSearch();
  });

  function toggleSelected(id: number, ev: MouseEvent) {
    const next = new Set(selected);
    if (ev.shiftKey && lastClickedId != null) {
      const a = items.findIndex((c) => c.id === lastClickedId);
      const b = items.findIndex((c) => c.id === id);
      if (a >= 0 && b >= 0) {
        const [from, to] = a < b ? [a, b] : [b, a];
        for (let i = from; i <= to; i++) next.add(items[i].id);
      } else {
        if (next.has(id)) next.delete(id);
        else next.add(id);
      }
    } else {
      if (next.has(id)) next.delete(id);
      else next.add(id);
    }
    selected = next;
    lastClickedId = id;
  }

  let lastClickedId = $state<number | null>(null);

  function selectAllVisible() {
    const next = new Set(selected);
    for (const c of items) next.add(c.id);
    selected = next;
  }
  function clearSelection() {
    selected = new Set();
  }

  async function bulkSuspend() {
    if (selected.size === 0) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:suspend", {
        ids: Array.from(selected),
      });
      await runSearch();
    } finally {
      busy = false;
    }
  }
  async function bulkUnsuspend() {
    if (selected.size === 0) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:unsuspend", {
        ids: Array.from(selected),
      });
      await runSearch();
    } finally {
      busy = false;
    }
  }
  async function bulkBury() {
    if (selected.size === 0) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:bury", {
        ids: Array.from(selected),
        manual: true,
      });
      await runSearch();
    } finally {
      busy = false;
    }
  }
  async function bulkSetFlag(flag: number) {
    if (selected.size === 0) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:set_flag", {
        ids: Array.from(selected),
        flag,
      });
      await runSearch();
    } finally {
      busy = false;
    }
  }
  async function bulkSetDeck() {
    if (selected.size === 0 || bulkDeckTarget == null) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:set_deck", {
        ids: Array.from(selected),
        deckId: bulkDeckTarget,
      });
      bulkDeckPickerOpen = false;
      await runSearch();
    } finally {
      busy = false;
    }
  }

  function selectedNoteIds(): number[] {
    const noteIds = new Set<number>();
    for (const cardId of selected) {
      const card = items.find((c) => c.id === cardId);
      if (card) noteIds.add(card.nid);
    }
    return [...noteIds];
  }

  async function bulkApplyTags() {
    const noteIds = selectedNoteIds();
    const tags = bulkTagInput
      .trim()
      .split(/\s+/)
      .filter((t) => t.length > 0);
    if (noteIds.length === 0 || tags.length === 0) return;
    busy = true;
    try {
      const cmd =
        bulkTagMode === "add"
          ? "study:anki:tags:add_to_notes"
          : "study:anki:tags:remove_from_notes";
      await pluginInvoke<{ updated: number }>("study", cmd, {
        noteIds,
        tags,
      });
      bulkTagOpen = false;
      bulkTagInput = "";
      await runSearch();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function unburyDeckAction() {
    if (unburyDeckTarget == null) return;
    busy = true;
    try {
      await pluginInvoke("study", "study:anki:cards:unbury_deck", {
        deckId: unburyDeckTarget,
      });
      unburyDeckPickerOpen = false;
      await runSearch();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function startEditNote(note: Note) {
    editNoteTarget = note;
    editNoteFields = [...note.fields];
    editNoteTags = note.tags.join(" ");
    editNoteOpen = true;
  }

  async function saveEditedNote() {
    if (!editNoteTarget) return;
    editNoteBusy = true;
    try {
      const tags = editNoteTags.trim().split(/\s+/).filter((t) => t.length > 0);
      await pluginInvoke("study", "study:anki:notes:update", {
        id: editNoteTarget.id,
        fields: editNoteFields,
        tags,
      });
      editNoteOpen = false;
      editNoteTarget = null;
      await runSearch();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      editNoteBusy = false;
    }
  }

  function askDeleteNote(noteId: number) {
    deleteNoteTarget = noteId;
    confirmDeleteNoteOpen = true;
  }

  async function confirmDeleteNote() {
    if (deleteNoteTarget == null) return;
    deleteNoteBusy = true;
    try {
      await pluginInvoke("study", "study:anki:notes:delete", {
        id: deleteNoteTarget,
      });
      confirmDeleteNoteOpen = false;
      deleteNoteTarget = null;
      drawerOpen = false;
      await runSearch();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      deleteNoteBusy = false;
    }
  }

  async function loadSiblingCards(noteId: number) {
    siblingsLoading = true;
    try {
      const cards = await pluginInvoke<Card[]>(
        "study",
        "study:anki:cards:list_by_note",
        { noteId },
      );
      siblingCards = cards ?? [];
    } catch (e) {
      siblingCards = [];
    } finally {
      siblingsLoading = false;
    }
  }
  function askDelete() {
    if (selected.size === 0) return;
    confirmAction = "delete";
    confirmOpen = true;
  }
  async function confirmAndDo() {
    if (confirmAction === "delete") {
      busy = true;
      try {
        await pluginInvoke("study", "study:anki:cards:delete", {
          ids: Array.from(selected),
        });
        await runSearch();
      } finally {
        busy = false;
      }
    }
    confirmAction = null;
  }

  async function openDrawer(cardId: number) {
    drawerOpen = true;
    drawerCardId = cardId;
    drawerStats = null;
    drawerNote = null;
    drawerError = "";
    siblingCards = [];
    try {
      const stats = await pluginInvoke<CardStats>(
        "study",
        "study:anki:stats:card",
        { cardId },
      );
      drawerStats = stats;
      drawerNote = notesById[stats.note_id] ?? null;
      if (!drawerNote) {
        drawerNote = await pluginInvoke<Note>("study", "study:anki:notes:get", {
          id: stats.note_id,
        });
      }
      void loadSiblingCards(stats.note_id);
    } catch (e) {
      drawerError = e instanceof Error ? e.message : String(e);
    }
  }
  function closeDrawer() {
    drawerOpen = false;
    drawerCardId = null;
    drawerStats = null;
    drawerNote = null;
  }

  function nextPage() {
    if (offset + limit >= total) return;
    offset += limit;
    runSearch();
  }
  function prevPage() {
    if (offset === 0) return;
    offset = Math.max(0, offset - limit);
    runSearch();
  }
</script>

<section class="study-page">
  <PageHero title={$t("study.anki.sidebar.browse")} />

  <div class="search-row">
    <input
      type="search"
      class="search-input"
      bind:value={query}
      placeholder={$t("study.anki.browse.search_placeholder")}
      aria-label="Search query"
    />
    <span class="total">{$t("study.anki.browse.cards_total", { n: total })}</span>
  </div>

  <div class="presets">
    {#each presetQueries as p (p.q)}
      <button type="button" class="preset" onclick={() => applyPreset(p.q)}>
        {p.label}
      </button>
    {/each}
  </div>

  {#if selected.size > 0}
    <div class="bulk-bar" role="toolbar" aria-label={$t("study.anki.browse.bulk_aria")}>
      <span class="bulk-count">{$t("study.anki.browse.n_selected", { n: selected.size })}</span>
      <button type="button" class="bulk-btn" onclick={bulkSuspend} disabled={busy}>
        {$t("study.anki.browse.suspend")}
      </button>
      <button type="button" class="bulk-btn" onclick={bulkUnsuspend} disabled={busy}>
        {$t("study.anki.browse.unsuspend")}
      </button>
      <button type="button" class="bulk-btn" onclick={bulkBury} disabled={busy}>
        {$t("study.anki.browse.bury")}
      </button>
      <details class="flag-menu">
        <summary class="bulk-btn">{$t("study.anki.browse.flag")}</summary>
        <div class="flag-grid">
          {#each [
            [$t("study.anki.browse.flag_none"), 0, "transparent"],
            [$t("study.anki.browse.flag_red"), 1, "var(--error)"],
            [$t("study.anki.browse.flag_orange"), 2, "var(--warning, var(--accent))"],
            [$t("study.anki.browse.flag_green"), 3, "var(--success, var(--accent))"],
            [$t("study.anki.browse.flag_blue"), 4, "var(--accent)"],
          ] as [label, n, color] (n)}
            <button
              type="button"
              class="flag-pick"
              onclick={() => bulkSetFlag(n as number)}
              disabled={busy}
            >
              <span class="flag-swatch" style:background="{color}"></span>
              <span>{label}</span>
            </button>
          {/each}
        </div>
      </details>
      <button
        type="button"
        class="bulk-btn"
        onclick={() => (bulkDeckPickerOpen = true)}
        disabled={busy}
      >
        {$t("study.anki.decks.move")}
      </button>
      <button
        type="button"
        class="bulk-btn"
        onclick={() => { bulkTagMode = "add"; bulkTagOpen = true; }}
        disabled={busy}
      >
        {$t("study.anki.browse.add_tag")}
      </button>
      <button
        type="button"
        class="bulk-btn"
        onclick={() => { bulkTagMode = "remove"; bulkTagOpen = true; }}
        disabled={busy}
      >
        {$t("study.anki.browse.remove_tag")}
      </button>
      <button
        type="button"
        class="bulk-btn danger"
        onclick={askDelete}
        disabled={busy}
      >
        {$t("study.common.delete")}
      </button>
      <button type="button" class="bulk-btn ghost" onclick={clearSelection}>
        {$t("study.common.clear")}
      </button>
    </div>
  {/if}

  <div class="utility-row">
    <button
      type="button"
      class="util-btn"
      onclick={() => { unburyDeckTarget = null; unburyDeckPickerOpen = true; }}
      disabled={busy}
      title={$t("study.anki.browse.unbury_deck_hint")}
    >
      {$t("study.anki.browse.unbury_deck")}
    </button>
  </div>

  {#if loading}
    <p class="muted">{$t("study.anki.revlog.searching")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if items.length === 0}
    <p class="muted center">{$t("study.anki.browse.no_cards")}</p>
  {:else}
    <div class="table-wrap">
      <table class="card-table">
        <thead>
          <tr>
            <th class="cb-col">
              <input
                type="checkbox"
                aria-label={$t("study.anki.media.select_visible")}
                checked={items.every((c) => selected.has(c.id))}
                onchange={(e) => {
                  if ((e.target as HTMLInputElement).checked) selectAllVisible();
                  else clearSelection();
                }}
              />
            </th>
            <th class="flag-col"></th>
            <th>{$t("study.anki.browse.th_front")}</th>
            <th class="deck-col">{$t("study.anki.sidebar.decks")}</th>
            <th class="state-col">{$t("study.anki.browse.th_state")}</th>
            <th class="num">Ivl</th>
            <th class="num">Reps</th>
            <th class="num">{$t("study.anki.settings.leech_hint").split(" ")[0] === "lapsos" ? "Lapsos" : "Lapses"}</th>
            <th class="date-col">{$t("study.anki.browse.th_modified")}</th>
          </tr>
        </thead>
        <tbody>
          {#each items as c (c.id)}
            {@const note = notesById[c.nid]}
            {@const sfld = note?.fields?.[0] ?? ""}
            {@const checked = selected.has(c.id)}
            <tr
              class:selected={checked}
              onclick={() => openDrawer(c.id)}
            >
              <td class="cb-col" onclick={(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  {checked}
                  onclick={(e) => {
                    e.stopPropagation();
                    toggleSelected(c.id, e as MouseEvent);
                  }}
                />
              </td>
              <td class="flag-col">
                <span class="flag-dot" style:background="{flagDot(c.flag)}"></span>
              </td>
              <td class="front">{strip(sfld) || `card ${c.id}`}</td>
              <td class="deck">{decksById[c.did] ?? c.did}</td>
              <td class="state">{queueLabel(c.queue)}</td>
              <td class="num mono">{c.ivl}</td>
              <td class="num mono">{c.reps}</td>
              <td class="num mono">{c.lapses}</td>
              <td class="date mono">{fmtRelativeSecs(c.mod_secs)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <div class="pagination">
      <button
        type="button"
        class="page-btn"
        onclick={prevPage}
        disabled={offset === 0 || loading}
      >
        ← Anterior
      </button>
      <span class="page-info">
        {offset + 1}–{Math.min(offset + limit, total)} de {total}
      </span>
      <button
        type="button"
        class="page-btn"
        onclick={nextPage}
        disabled={offset + limit >= total || loading}
      >
        Próxima →
      </button>
    </div>
  {/if}
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  title={$t("study.anki.browse.delete_cards_title")}
  message={$t("study.anki.browse.delete_cards_confirm", { n: selected.size })}
  confirmLabel={$t("study.common.delete")}
  variant="danger"
  onConfirm={confirmAndDo}
/>

{#if bulkDeckPickerOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) bulkDeckPickerOpen = false;
    }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>Mover {selected.size} cards</h3>
      <label>
        <span>{$t("study.anki.decks.new_deck")}</span>
        <select bind:value={bulkDeckTarget}>
          {#each allDecks.filter((d) => !d.filtered) as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </label>
      <div class="modal-actions">
        <button
          type="button"
          class="btn-secondary"
          onclick={() => (bulkDeckPickerOpen = false)}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={bulkSetDeck}
          disabled={busy || bulkDeckTarget == null}
        >
          Mover
        </button>
      </div>
    </div>
  </div>
{/if}

{#if bulkTagOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) bulkTagOpen = false; }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>
        {bulkTagMode === "add" ? $t("study.anki.browse.add_tag") : $t("study.anki.browse.remove_tag")}
        em {selectedNoteIds().length}
        {$t("study.anki.browse.notes_count", { n: selectedNoteIds().length })}
      </h3>
      <label>
        <span>{$t("study.anki.browse.tags_input_hint")}</span>
        <input
          type="text"
          bind:value={bulkTagInput}
          placeholder="exemplo livro::cap1"
          onkeydown={(e) => { if (e.key === "Enter") bulkApplyTags(); }}
        />
      </label>
      <div class="modal-actions">
        <button
          type="button"
          class="btn-secondary"
          onclick={() => (bulkTagOpen = false)}
          disabled={busy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={bulkApplyTags}
          disabled={busy || bulkTagInput.trim() === ""}
        >
          {bulkTagMode === "add" ? "Adicionar" : "Remover"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if unburyDeckPickerOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) unburyDeckPickerOpen = false; }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>Reativar cards enterrados</h3>
      <p class="modal-hint">
        Reverte o estado de enterro de cards do deck selecionado, retornando-os
        às filas normais.
      </p>
      <label>
        <span>Deck</span>
        <select bind:value={unburyDeckTarget}>
          <option value={null}>Selecione…</option>
          {#each allDecks as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </label>
      <div class="modal-actions">
        <button
          type="button"
          class="btn-secondary"
          onclick={() => (unburyDeckPickerOpen = false)}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={unburyDeckAction}
          disabled={busy || unburyDeckTarget == null}
        >
          Reativar
        </button>
      </div>
    </div>
  </div>
{/if}

{#if editNoteOpen && editNoteTarget}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) editNoteOpen = false; }}
  >
    <div class="modal modal-wide" role="dialog" aria-modal="true">
      <h3>Editar nota #{editNoteTarget.id}</h3>
      <div class="edit-fields">
        {#each editNoteFields as _field, idx (idx)}
          <label class="edit-field">
            <span>Campo {idx + 1}</span>
            <textarea bind:value={editNoteFields[idx]} rows="2"></textarea>
          </label>
        {/each}
        <label class="edit-field">
          <span>Tags (espaço como separador)</span>
          <input type="text" bind:value={editNoteTags} />
        </label>
      </div>
      <div class="modal-actions">
        <button
          type="button"
          class="btn-secondary"
          onclick={() => (editNoteOpen = false)}
          disabled={editNoteBusy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={saveEditedNote}
          disabled={editNoteBusy}
        >
          {editNoteBusy ? $t("study.common.saving") : $t("study.common.save")}
        </button>
      </div>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmDeleteNoteOpen}
  title={$t("study.anki.browse.delete_note_title")}
  message={$t("study.anki.browse.delete_note_confirm")}
  confirmLabel={$t("study.anki.browse.delete_note_confirm_label")}
  variant="danger"
  onConfirm={confirmDeleteNote}
/>

{#if drawerOpen}
  <aside
    class="drawer-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeDrawer();
    }}
  >
    <div class="drawer" role="dialog" aria-modal="true">
      <header class="drawer-head">
        <h3>Card #{drawerCardId}</h3>
        <button type="button" class="close-btn" onclick={closeDrawer} aria-label="Fechar">×</button>
      </header>
      {#if drawerError}
        <p class="error">{drawerError}</p>
      {:else if !drawerStats}
        <p class="muted">Carregando…</p>
      {:else}
        <section class="drawer-section">
          <div class="drawer-section-head">
            <h4>Conteúdo</h4>
            {#if drawerNote}
              <div class="drawer-section-actions">
                <button
                  type="button"
                  class="btn-link"
                  onclick={() => drawerNote && startEditNote(drawerNote)}
                >
                  Editar
                </button>
                <button
                  type="button"
                  class="btn-link danger"
                  onclick={() => drawerNote && askDeleteNote(drawerNote.id)}
                >
                  Excluir nota
                </button>
              </div>
            {/if}
          </div>
          {#if drawerNote}
            <ul class="fields">
              {#each drawerNote.fields as field, i (i)}
                <li>
                  <span class="field-idx">#{i}</span>
                  <span class="field-val">{strip(field) || "—"}</span>
                </li>
              {/each}
            </ul>
            {#if drawerNote.tags.length > 0}
              <div class="tag-row">
                {#each drawerNote.tags as t (t)}
                  <span class="tag">{t}</span>
                {/each}
              </div>
            {/if}
          {/if}
        </section>

        <section class="drawer-section">
          <h4>Outras cards desta nota</h4>
          {#if siblingsLoading}
            <p class="muted small">Carregando…</p>
          {:else if siblingCards.length <= 1}
            <p class="muted small">Esta nota tem só essa card.</p>
          {:else}
            <ul class="sibling-list">
              {#each siblingCards as sib (sib.id)}
                <li class:current={sib.id === drawerCardId}>
                  <button
                    type="button"
                    class="sibling-link"
                    onclick={() => sib.id !== drawerCardId && openDrawer(sib.id)}
                    disabled={sib.id === drawerCardId}
                  >
                    <span class="sib-ord">ord {sib.ord}</span>
                    <span class="sib-state">{sib.queue}</span>
                    <span class="sib-id mono">#{sib.id}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <section class="drawer-section">
          <h4>Scheduler</h4>
          <dl class="kv">
            <dt>Intervalo</dt>
            <dd class="mono">{drawerStats.interval_days}d</dd>
            <dt>Ease factor</dt>
            <dd class="mono">{(drawerStats.ease_factor / 1000).toFixed(2)}</dd>
            <dt>Reviews</dt>
            <dd class="mono">{drawerStats.reviews_count}</dd>
            <dt>Lapsos</dt>
            <dd class="mono">{drawerStats.lapses_count}</dd>
            <dt>Tempo médio</dt>
            <dd class="mono">{drawerStats.avg_seconds.toFixed(1)}s</dd>
            <dt>Primeiro review</dt>
            <dd class="mono">{fmtDate(drawerStats.first_review_ms)}</dd>
            <dt>Último review</dt>
            <dd class="mono">{fmtDate(drawerStats.latest_review_ms)}</dd>
          </dl>
        </section>

        {#if drawerStats.fsrs_memory_state}
          <section class="drawer-section">
            <h4>FSRS</h4>
            <dl class="kv">
              <dt>Stability</dt>
              <dd class="mono">{drawerStats.fsrs_memory_state.stability.toFixed(2)}</dd>
              <dt>Difficulty</dt>
              <dd class="mono">{drawerStats.fsrs_memory_state.difficulty.toFixed(2)}</dd>
              {#if drawerStats.fsrs_retrievability != null}
                <dt>Retrievability</dt>
                <dd class="mono">{(drawerStats.fsrs_retrievability * 100).toFixed(1)}%</dd>
              {/if}
            </dl>
          </section>
        {/if}

        {#if drawerStats.revlog.length > 0}
          <section class="drawer-section">
            <h4>Histórico ({drawerStats.revlog.length})</h4>
            <ul class="revlog-list">
              {#each [...drawerStats.revlog].reverse().slice(0, 30) as r (r.id)}
                <li>
                  <span class="rev-date mono">{fmtDate(r.id)}</span>
                  <span class="rev-ease mono">btn {r.ease}</span>
                  <span class="rev-ivl mono">{r.last_ivl}d → {r.ivl}d</span>
                  <span class="rev-kind">{r.kind}</span>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      {/if}
    </div>
  </aside>
{/if}

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    width: 100%;
    max-width: 1200px;
    margin-inline: auto;
  }
  .muted {
    color: var(--tertiary);
    font-size: 13px;
  }
  .muted.center {
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

  .search-row {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }
  .search-input {
    flex: 1;
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 10px 14px;
    border-radius: var(--border-radius);
    font-family: var(--font-mono, monospace);
    font-size: 13px;
  }
  .search-input:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: -1px;
  }
  .total {
    font-size: 12px;
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    min-width: 80px;
    text-align: right;
  }

  .presets {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .preset {
    background: transparent;
    border: none;
    color: var(--tertiary);
    padding: 4px 12px;
    border-radius: 999px;
    font-size: 12px;
    cursor: pointer;
    transition:
      color 150ms ease,
      border-color 150ms ease;
  }
  .preset:hover {
    color: var(--secondary);
    border-color: var(--accent);
  }
  .preset:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }

  .bulk-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--accent) 25%, var(--input-border));
    border-radius: var(--border-radius);
    flex-wrap: wrap;
  }
  .bulk-count {
    font-size: 12px;
    color: var(--accent);
    font-weight: 500;
  }
  .bulk-btn {
    background: transparent;
    border: none;
    color: var(--secondary);
    padding: 4px 12px;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .bulk-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .bulk-btn:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .bulk-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .bulk-btn.danger:hover:not(:disabled) {
    border-color: var(--error);
    color: var(--error);
  }
  .bulk-btn.ghost {
    margin-left: auto;
  }
  .flag-menu {
    position: relative;
  }
  .flag-menu summary {
    list-style: none;
  }
  .flag-menu summary::-webkit-details-marker {
    display: none;
  }
  .flag-grid {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    z-index: 10;
    min-width: 140px;
    box-shadow: 0 8px 32px color-mix(in oklab, var(--bg, black) 30%, transparent);
  }
  .flag-pick {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: transparent;
    border: 0;
    color: var(--secondary);
    font-size: 12px;
    cursor: pointer;
    border-radius: 4px;
    text-align: left;
  }
  .flag-pick:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .flag-swatch {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: none;
  }

  .table-wrap {
    overflow-x: auto;
    border: none;
    border-radius: var(--border-radius);
  }
  .card-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .card-table th,
  .card-table td {
    padding: 8px 10px;
    text-align: left;
    border-bottom: none;
  }
  .card-table th {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    font-weight: 500;
    background: var(--button-elevated);
    position: sticky;
    top: 0;
  }
  .card-table tbody tr {
    cursor: pointer;
    transition: background 150ms ease;
  }
  .card-table tbody tr:hover {
    background: color-mix(in oklab, var(--accent) 4%, transparent);
  }
  .card-table tbody tr.selected {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .cb-col {
    width: 28px;
  }
  .flag-col {
    width: 22px;
  }
  .flag-dot {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: none;
  }
  .deck-col {
    max-width: 160px;
  }
  .state-col {
    width: 90px;
  }
  .date-col {
    width: 90px;
  }
  .num {
    text-align: right;
    width: 60px;
  }
  .mono {
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
  }
  .front {
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 0;
  }
  .deck,
  .state,
  .date {
    color: var(--tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pagination {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }
  .page-btn {
    background: transparent;
    border: none;
    color: var(--secondary);
    padding: 6px 14px;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .page-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .page-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .page-info {
    font-size: 12px;
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, var(--bg, black) 60%, transparent);
    backdrop-filter: blur(6px);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
  }
  .modal {
    width: min(440px, 100%);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 2);
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }
  .modal h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
    color: var(--secondary);
  }
  .modal label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .modal select {
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 6px 10px;
    border-radius: var(--border-radius);
    font-size: 13px;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .modal-wide {
    max-width: 720px;
  }
  .modal-hint {
    margin: 0 0 12px;
    font-size: 12px;
    color: var(--tertiary);
  }
  .edit-fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 16px;
    max-height: 60vh;
    overflow-y: auto;
  }
  .edit-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .edit-field textarea,
  .edit-field input {
    padding: 8px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    resize: vertical;
  }
  .edit-field textarea:focus,
  .edit-field input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .utility-row {
    display: flex;
    gap: 8px;
    padding-top: 4px;
  }
  .util-btn {
    padding: 5px 12px;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .util-btn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--accent);
  }
  .util-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .drawer-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .drawer-section-actions {
    display: flex;
    gap: 4px;
  }
  .btn-link {
    padding: 2px 8px;
    background: transparent;
    border: 0;
    color: var(--accent);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    border-radius: 4px;
  }
  .btn-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .btn-link.danger {
    color: var(--error, var(--accent));
  }
  .btn-link.danger:hover {
    background: color-mix(in oklab, var(--error, var(--accent)) 10%, transparent);
  }

  .sibling-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sibling-list li.current .sibling-link {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    cursor: default;
  }
  .sibling-link {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    background: transparent;
    border-radius: var(--border-radius);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .sibling-link:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .sib-ord {
    font-size: 12px;
  }
  .sib-state {
    font-size: 11px;
    color: var(--tertiary);
  }
  .sib-id {
    margin-left: auto;
    font-size: 11px;
    color: var(--tertiary);
  }
  .small {
    font-size: 12px;
  }
  .btn-primary {
    padding: 8px 18px;
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border: 0;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-secondary {
    padding: 8px 16px;
    background: transparent;
    border: none;
    color: var(--secondary);
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .btn-secondary:hover {
    border-color: var(--accent);
  }

  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, var(--bg, black) 50%, transparent);
    z-index: 50;
    display: flex;
    justify-content: flex-end;
  }
  .drawer {
    width: min(480px, 100%);
    height: 100%;
    background: var(--button-elevated);
    border-left: none;
    overflow-y: auto;
    padding: calc(var(--padding) * 2);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }
  .drawer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
    font-family: var(--font-mono, monospace);
  }
  .close-btn {
    background: transparent;
    border: 0;
    color: var(--tertiary);
    font-size: 20px;
    cursor: pointer;
    padding: 0 8px;
  }
  .close-btn:hover {
    color: var(--secondary);
  }
  .drawer-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .drawer-section h4 {
    margin: 0;
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
  }
  .fields {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .fields li {
    display: flex;
    gap: 10px;
    padding: 8px 10px;
    background: var(--bg);
    border: none;
    border-radius: var(--border-radius);
    font-size: 13px;
  }
  .field-idx {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    color: var(--tertiary);
    flex-shrink: 0;
  }
  .field-val {
    color: var(--secondary);
    word-break: break-word;
  }
  .tag-row {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .tag {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .kv {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px 12px;
    margin: 0;
    font-size: 12px;
  }
  .kv dt {
    color: var(--tertiary);
  }
  .kv dd {
    margin: 0;
    color: var(--secondary);
  }
  .revlog-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .revlog-list li {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    gap: 8px;
    padding: 4px 8px;
    background: var(--bg);
    border-radius: 4px;
  }
  .rev-date {
    color: var(--tertiary);
  }
  .rev-ease {
    color: var(--accent);
  }
  .rev-ivl {
    color: var(--secondary);
  }
  .rev-kind {
    color: var(--tertiary);
    font-size: 11px;
  }

  @media (max-width: 720px) {
    .deck-col,
    .date-col {
      display: none;
    }
    .drawer {
      width: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .preset,
    .card-table tbody tr {
      transition: none;
    }
  }
</style>
