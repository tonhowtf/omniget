<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type DeckTreeNode = {
    deck_id: number;
    name: string;
    full_name: string;
    level: number;
    collapsed: boolean;
    filtered: boolean;
    children: DeckTreeNode[];
  };

  type DeckSummary = {
    id: number;
    name: string;
    filtered: boolean;
    config_id: number | null;
  };

  type DeckConfigData = {
    learn_steps: number[];
    relearn_steps: number[];
    new_per_day: number;
    reviews_per_day: number;
    leech_threshold: number;
    desired_retention: number;
    initial_ease: number;
    [k: string]: unknown;
  };

  type DeckConfig = {
    id: number;
    name: string;
    mtime_secs: number;
    usn: number;
    config: DeckConfigData;
  };

  let loading = $state(true);
  let error = $state("");
  let tree = $state<DeckTreeNode[]>([]);
  let allDecks = $state<DeckSummary[]>([]);
  let countsByDeck = $state<Record<number, number>>({});

  let editingId = $state<number | null>(null);
  let editingValue = $state("");

  let confirmOpen = $state(false);
  let pendingDelete = $state<DeckTreeNode | null>(null);

  let configOpen = $state(false);
  let configDeckId = $state<number | null>(null);
  let configData = $state<DeckConfig | null>(null);
  let configBusy = $state(false);
  let configError = $state("");

  let reparentOpen = $state(false);
  let reparentDeck = $state<DeckTreeNode | null>(null);
  let reparentTarget = $state<number | null>(null);

  let createOpen = $state(false);
  let createName = $state("");
  let createParentId = $state<number | null>(null);
  let createBusy = $state(false);

  function flatten(nodes: DeckTreeNode[]): DeckTreeNode[] {
    const out: DeckTreeNode[] = [];
    function walk(list: DeckTreeNode[]) {
      for (const n of list) {
        out.push(n);
        if (n.children?.length) walk(n.children);
      }
    }
    walk(nodes);
    return out;
  }

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const [t, list] = await Promise.all([
        pluginInvoke<DeckTreeNode[]>("study", "study:anki:decks:tree"),
        pluginInvoke<DeckSummary[]>("study", "study:anki:decks:list"),
      ]);
      tree = t;
      allDecks = list;

      const flat = flatten(tree);
      const results = await Promise.all(
        flat.map((d) =>
          pluginInvoke<{ items: unknown[]; total: number }>(
            "study",
            "study:anki:search:cards",
            {
              query: `"deck:${d.full_name}"`,
              limit: 0,
              offset: 0,
            },
          )
            .then((r) => [d.deck_id, r.total ?? 0] as const)
            .catch(() => [d.deck_id, 0] as const),
        ),
      );
      countsByDeck = Object.fromEntries(results);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  function startRename(node: DeckTreeNode) {
    editingId = node.deck_id;
    const segments = node.full_name.split("::");
    editingValue = segments[segments.length - 1] ?? node.name;
  }

  async function commitRename(node: DeckTreeNode) {
    const trimmed = editingValue.trim();
    editingId = null;
    if (!trimmed) return;
    const segments = node.full_name.split("::");
    segments[segments.length - 1] = trimmed;
    const newName = segments.join("::");
    if (newName === node.full_name) return;
    try {
      await pluginInvoke("study", "study:anki:decks:rename", {
        id: node.deck_id,
        newName,
      });
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function cancelRename() {
    editingId = null;
    editingValue = "";
  }

  function askDelete(node: DeckTreeNode) {
    pendingDelete = node;
    confirmOpen = true;
  }

  async function confirmDelete() {
    const node = pendingDelete;
    pendingDelete = null;
    if (!node) return;
    try {
      await pluginInvoke("study", "study:anki:decks:delete", { id: node.deck_id });
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function openConfig(deckId: number) {
    configDeckId = deckId;
    configData = null;
    configError = "";
    configOpen = true;
    try {
      const cfg = await pluginInvoke<DeckConfig>(
        "study",
        "study:anki:config:get_deck",
        { deckId },
      );
      configData = cfg;
    } catch (e) {
      configError = e instanceof Error ? e.message : String(e);
    }
  }

  function closeConfig() {
    configOpen = false;
    configDeckId = null;
    configData = null;
  }

  async function saveConfig() {
    if (!configData) return;
    configBusy = true;
    configError = "";
    try {
      await pluginInvoke("study", "study:anki:config:set_deck", {
        config: configData,
      });
      configOpen = false;
      configData = null;
    } catch (e) {
      configError = e instanceof Error ? e.message : String(e);
    } finally {
      configBusy = false;
    }
  }

  function openReparent(node: DeckTreeNode) {
    reparentDeck = node;
    reparentTarget = null;
    reparentOpen = true;
  }

  function closeReparent() {
    reparentOpen = false;
    reparentDeck = null;
    reparentTarget = null;
  }

  async function commitReparent() {
    if (!reparentDeck) return;
    try {
      await pluginInvoke("study", "study:anki:decks:reparent", {
        id: reparentDeck.deck_id,
        newParentId: reparentTarget,
      });
      closeReparent();
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function openCreate(parentId: number | null) {
    createParentId = parentId;
    createName = "";
    createOpen = true;
  }

  function closeCreate() {
    createOpen = false;
    createName = "";
  }

  async function commitCreate() {
    const trimmed = createName.trim();
    if (!trimmed) return;
    createBusy = true;
    try {
      let fullName = trimmed;
      if (createParentId != null) {
        const parent = allDecks.find((d) => d.id === createParentId);
        if (parent) fullName = `${parent.name}::${trimmed}`;
      }
      await pluginInvoke("study", "study:anki:decks:create", { name: fullName });
      closeCreate();
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      createBusy = false;
    }
  }

  function setStep(arr: number[], idx: number, value: string): number[] {
    const next = [...arr];
    const num = Number.parseFloat(value);
    next[idx] = Number.isFinite(num) ? num : 0;
    return next;
  }

  function fmtSteps(arr: number[]): string {
    return arr.map((n) => `${n}m`).join(" ");
  }

  function parseSteps(text: string): number[] {
    return text
      .split(/[\s,]+/)
      .map((s) => s.replace(/[^0-9.]/g, ""))
      .map((s) => Number.parseFloat(s))
      .filter((n) => Number.isFinite(n) && n > 0);
  }

  const reparentOptions = $derived.by(() => {
    if (!reparentDeck) return [] as DeckSummary[];
    const blockedPrefix = reparentDeck.full_name + "::";
    return allDecks.filter(
      (d) =>
        !d.filtered &&
        d.id !== reparentDeck!.deck_id &&
        !d.name.startsWith(blockedPrefix),
    );
  });

  const createParentOptions = $derived(
    allDecks.filter((d) => !d.filtered),
  );
</script>

<section class="study-page">
  <PageHero title={$t("study.anki.decks.title")} />

  {#if loading}
    <p class="muted">{$t("study.anki.decks.loading_tree")}</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else}
    <div class="toolbar">
      <button type="button" class="btn-primary" onclick={() => openCreate(null)}>
        + {$t("study.anki.decks.new_deck")}
      </button>
      <a href="/study/anki/decks/filtered" class="advanced-link">{$t("study.anki.decks.filtered_decks")}</a>
      <a href="/study/anki/decks/presets" class="advanced-link">{$t("study.anki.decks.presets")}</a>
      <span class="hint">
        {$t("study.anki.decks.subdeck_hint_a")} <code>::</code> {$t("study.anki.decks.subdeck_hint_b")} <code>Idiomas::Espanhol</code>)
      </span>
    </div>

    <ul class="tree-root">
      {@render renderNodes(tree)}
    </ul>
  {/if}
</section>

{#snippet renderNodes(nodes: DeckTreeNode[])}
  {#each nodes as node (node.deck_id)}
    {@const count = countsByDeck[node.deck_id] ?? 0}
    <li class="node" data-level={node.level}>
      <div class="row" style:padding-left="{node.level * 16}px">
        <div class="row-main">
          {#if editingId === node.deck_id}
            <input
              class="rename-input"
              bind:value={editingValue}
              onblur={() => commitRename(node)}
              onkeydown={(e) => {
                if (e.key === "Enter") commitRename(node);
                if (e.key === "Escape") cancelRename();
              }}
              autofocus
            />
          {:else}
            <button
              type="button"
              class="name-btn"
              onclick={() => startRename(node)}
              aria-label={$t("study.anki.decks.rename_aria", { name: node.name })}
            >
              <span class="name">{node.name}</span>
              {#if node.filtered}
                <span class="filtered-tag">filtered</span>
              {/if}
            </button>
          {/if}
          <span class="count" title="{count} cards">{count}</span>
        </div>
        <div class="actions">
          <a class="action study" href="/study/anki/study/{node.deck_id}">{$t("study.anki.decks.study")}</a>
          <button
            type="button"
            class="action"
            onclick={() => openCreate(node.deck_id)}
            title={$t("study.anki.decks.create_subdeck")}
          >
            + sub
          </button>
          <button
            type="button"
            class="action"
            onclick={() => openReparent(node)}
            title={$t("study.anki.decks.move_deck")}
            disabled={node.deck_id === 1}
          >
            mover
          </button>
          <button
            type="button"
            class="action"
            onclick={() => openConfig(node.deck_id)}
            title={$t("study.anki.decks.configure_deck")}
            disabled={node.filtered}
          >
            config
          </button>
          <button
            type="button"
            class="action danger"
            onclick={() => askDelete(node)}
            title={$t("study.anki.decks.delete_deck")}
            disabled={node.deck_id === 1}
          >
            {$t("study.common.delete")}
          </button>
        </div>
      </div>
      {#if node.children.length > 0}
        <ul class="tree-sub">
          {@render renderNodes(node.children)}
        </ul>
      {/if}
    </li>
  {/each}
{/snippet}

<ConfirmDialog
  bind:open={confirmOpen}
  title={$t("study.anki.decks.delete_deck")}
  message={pendingDelete
    ? $t("study.anki.decks.delete_confirm", { name: pendingDelete.name })
    : ""}
  confirmLabel={$t("study.common.delete")}
  variant="danger"
  onConfirm={confirmDelete}
/>

{#if reparentOpen && reparentDeck}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeReparent();
    }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>{$t("study.anki.decks.move_title", { name: reparentDeck.name })}</h3>
      <label>
        <span>{$t("study.anki.decks.new_parent")}</span>
        <select bind:value={reparentTarget}>
          <option value={null}>{$t("study.anki.decks.root")}</option>
          {#each reparentOptions as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </label>
      <div class="modal-actions">
        <button type="button" class="btn-secondary" onclick={closeReparent}>
          {$t("study.common.cancel")}
        </button>
        <button type="button" class="btn-primary" onclick={commitReparent}>
          {$t("study.anki.decks.move")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if createOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeCreate();
    }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <h3>{$t("study.anki.decks.new_deck_title")}</h3>
      {#if createParentId != null}
        {@const parent = allDecks.find((d) => d.id === createParentId)}
        {#if parent}
          <p class="muted small">
            {$t("study.anki.decks.created_inside")} <code>{parent.name}</code>
          </p>
        {/if}
      {/if}
      <label>
        <span>{$t("study.anki.notetypes.name_label")}</span>
        <input
          type="text"
          bind:value={createName}
          placeholder="Espanhol"
          onkeydown={(e) => {
            if (e.key === "Enter") commitCreate();
          }}
          autofocus
        />
      </label>
      <div class="modal-actions">
        <button type="button" class="btn-secondary" onclick={closeCreate}>
          {$t("study.common.cancel")}
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={commitCreate}
          disabled={createBusy || !createName.trim()}
        >
          {createBusy ? $t("study.notes.creating") : $t("study.common.create")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if configOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeConfig();
    }}
  >
    <div class="modal config-modal" role="dialog" aria-modal="true">
      <header class="modal-head">
        <h3>{$t("study.anki.decks.config_title")}</h3>
        {#if configData}
          <small class="muted">{configData.name}</small>
        {/if}
      </header>
      {#if configError}
        <p class="error small">{configError}</p>
      {/if}
      {#if !configData}
        <p class="muted">{$t("study.anki.settings.loading")}</p>
      {:else}
        <div class="config-grid">
          <label>
            <span>{$t("study.anki.settings.new_per_day")}</span>
            <input
              type="number"
              min="0"
              bind:value={configData.config.new_per_day}
            />
          </label>
          <label>
            <span>{$t("study.anki.settings.rev_per_day")}</span>
            <input
              type="number"
              min="0"
              bind:value={configData.config.reviews_per_day}
            />
          </label>
          <label>
            <span>{$t("study.anki.settings.leech_threshold")}</span>
            <input
              type="number"
              min="0"
              bind:value={configData.config.leech_threshold}
            />
          </label>
          <label>
            <span>{$t("study.anki.settings.retention")}</span>
            <input
              type="number"
              min="0.5"
              max="0.99"
              step="0.01"
              bind:value={configData.config.desired_retention}
            />
          </label>
          <label>
            <span>{$t("study.anki.decks.initial_ease")}</span>
            <input
              type="number"
              min="1.3"
              step="0.05"
              bind:value={configData.config.initial_ease}
            />
          </label>
          <label class="span-2">
            <span>{$t("study.anki.decks.learn_steps_space")}</span>
            <input
              type="text"
              value={fmtSteps(configData.config.learn_steps)}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                if (configData) configData.config.learn_steps = parseSteps(v);
              }}
              placeholder="1 10"
            />
          </label>
          <label class="span-2">
            <span>{$t("study.anki.decks.relearn_steps_min")}</span>
            <input
              type="text"
              value={fmtSteps(configData.config.relearn_steps)}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                if (configData) configData.config.relearn_steps = parseSteps(v);
              }}
              placeholder="10"
            />
          </label>
        </div>
      {/if}
      <div class="modal-actions">
        <button type="button" class="btn-secondary" onclick={closeConfig}>
          Cancelar
        </button>
        <button
          type="button"
          class="btn-primary"
          onclick={saveConfig}
          disabled={configBusy || !configData}
        >
          {configBusy ? $t("study.common.saving") : $t("study.common.save")}
        </button>
      </div>
    </div>
  </div>
{/if}

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
    font-size: 13px;
  }
  .muted.small {
    font-size: 12px;
    margin: 4px 0 0;
  }
  .error {
    color: var(--error);
    font-size: 13px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--error) 8%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 25%, var(--input-border));
    border-radius: var(--border-radius);
  }
  .error.small {
    padding: 6px 10px;
    font-size: 12px;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--padding);
    flex-wrap: wrap;
  }
  .hint {
    font-size: 12px;
    color: var(--tertiary);
  }
  .hint code {
    font-family: var(--font-mono, monospace);
    background: var(--input-bg);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .tree-root,
  .tree-sub {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--button-elevated);
    transition: border-color 150ms ease;
  }
  .row:hover {
    border-color: color-mix(in oklab, var(--accent) 30%, var(--input-border));
  }
  .row-main {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }
  .name-btn {
    background: transparent;
    border: 0;
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: text;
    padding: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .name-btn:hover .name {
    color: var(--accent);
  }
  .name-btn:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
    border-radius: 4px;
  }
  .filtered-tag {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .rename-input {
    flex: 1;
    background: var(--input-bg);
    border: 1px solid var(--accent);
    color: var(--secondary);
    padding: 4px 8px;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
  }
  .count {
    font-family: var(--font-mono, monospace);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--tertiary);
    min-width: 36px;
    text-align: right;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .action {
    display: inline-flex;
    align-items: center;
    background: transparent;
    border: none;
    color: var(--tertiary);
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
    text-decoration: none;
    transition:
      background 150ms ease,
      color 150ms ease,
      border-color 150ms ease;
  }
  .action:hover:not(:disabled) {
    color: var(--secondary);
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .action:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .action.study {
    color: var(--accent);
    border-color: color-mix(in oklab, var(--accent) 35%, var(--input-border));
  }
  .action.danger:hover:not(:disabled) {
    color: var(--error);
    border-color: var(--error);
    background: color-mix(in oklab, var(--error) 8%, transparent);
  }

  .advanced-link {
    color: var(--accent);
    font-size: 12px;
    text-decoration: none;
    padding: 4px 8px;
    border-radius: var(--border-radius);
  }
  .advanced-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 18px;
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border: 0;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: filter 150ms ease;
  }
  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn-primary:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 2px;
  }
  .btn-primary:disabled {
    background: color-mix(in oklab, var(--input-border) 80%, transparent);
    color: var(--tertiary);
    cursor: not-allowed;
  }
  .btn-secondary {
    display: inline-flex;
    align-items: center;
    padding: 8px 16px;
    background: transparent;
    border: none;
    color: var(--secondary);
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
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
  .config-modal {
    width: min(640px, 100%);
    max-height: 90vh;
    overflow-y: auto;
  }
  .modal h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
    color: var(--secondary);
  }
  .modal-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .modal label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .modal label.span-2 {
    grid-column: 1 / -1;
  }
  .modal input,
  .modal select {
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    padding: 6px 10px;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 13px;
  }
  .modal input:focus-visible,
  .modal select:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: -1px;
  }
  .modal input[type="number"] {
    font-family: var(--font-mono, monospace);
  }
  .config-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: var(--padding);
  }

  @media (max-width: 600px) {
    .config-grid {
      grid-template-columns: 1fr;
    }
    .config-grid .span-2 {
      grid-column: auto;
    }
    .actions {
      gap: 2px;
    }
    .action {
      padding: 4px 8px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .row,
    .action,
    .btn-primary,
    .btn-secondary {
      transition: none;
    }
  }
</style>
