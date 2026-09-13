<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type DeckConfigSummary = {
    id: number;
    name: string;
    mtime_secs: number;
    use_count: number;
  };

  type DeckConfigData = {
    learn_steps: number[];
    relearn_steps: number[];
    fsrs_params_5: number[];
    new_per_day: number;
    reviews_per_day: number;
    initial_ease: number;
    easy_multiplier: number;
    hard_multiplier: number;
    lapse_multiplier: number;
    interval_multiplier: number;
    maximum_review_interval: number;
    minimum_lapse_interval: number;
    graduating_interval_good: number;
    graduating_interval_easy: number;
    leech_threshold: number;
    cap_answer_time_to_secs: number;
    show_timer: boolean;
    stop_timer_on_answer: boolean;
    bury_new: boolean;
    bury_reviews: boolean;
    bury_interday_learning: boolean;
    disable_autoplay: boolean;
    desired_retention: number;
    historical_retention: number;
    ignore_revlogs_before_date: string;
    new_card_insert_order: string;
    new_card_gather_priority: string;
    new_card_sort_order: string;
    new_mix: string;
    review_order: string;
    interday_learning_mix: string;
    leech_action: string;
  };

  type DeckConfig = {
    id: number;
    name: string;
    mtime_secs: number;
    usn: number;
    config: DeckConfigData;
  };

  const DEFAULT_PRESET_ID = 1;

  let summaries = $state<DeckConfigSummary[]>([]);
  let loading = $state(true);
  let error = $state("");
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let createName = $state("");
  let creating = $state(false);

  let editing = $state<DeckConfig | null>(null);
  let editForm = $state<DeckConfigData | null>(null);
  let editLearnRaw = $state("");
  let editRelearnRaw = $state("");
  let editBusy = $state(false);

  let confirmDeleteOpen = $state(false);
  let deleteTarget = $state<DeckConfigSummary | null>(null);
  let deleteBusy = $state(false);

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      summaries = await pluginInvoke<DeckConfigSummary[]>(
        "study",
        "study:anki:deckconfig:list",
      );
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function create() {
    const name = createName.trim();
    if (!name) return;
    creating = true;
    try {
      await pluginInvoke<{ id: number }>(
        "study",
        "study:anki:deckconfig:create",
        { name },
      );
      showToast("ok", `Preset "${name}" criado`);
      createName = "";
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  async function startEdit(summary: DeckConfigSummary) {
    try {
      const full = await pluginInvoke<DeckConfig>(
        "study",
        "study:anki:deckconfig:get",
        { id: summary.id },
      );
      editing = full;
      editForm = { ...full.config };
      editLearnRaw = full.config.learn_steps.join(" ");
      editRelearnRaw = full.config.relearn_steps.join(" ");
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  function parseSteps(raw: string): number[] {
    return raw
      .trim()
      .split(/\s+/)
      .map((s) => Number(s))
      .filter((n) => Number.isFinite(n) && n > 0);
  }

  async function saveEdit() {
    if (!editing || !editForm) return;
    editBusy = true;
    try {
      const updated: DeckConfig = {
        ...editing,
        config: {
          ...editForm,
          learn_steps: parseSteps(editLearnRaw),
          relearn_steps: parseSteps(editRelearnRaw),
        },
      };
      await pluginInvoke("study", "study:anki:deckconfig:update", {
        config: updated,
      });
      showToast("ok", $t("study.anki.preset_updated"));
      editing = null;
      editForm = null;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      editBusy = false;
    }
  }

  function askDelete(summary: DeckConfigSummary) {
    if (summary.id === DEFAULT_PRESET_ID) return;
    deleteTarget = summary;
    confirmDeleteOpen = true;
  }

  async function confirmDelete() {
    if (!deleteTarget) return;
    deleteBusy = true;
    try {
      await pluginInvoke("study", "study:anki:deckconfig:delete", {
        id: deleteTarget.id,
      });
      showToast("ok", `Preset "${deleteTarget.name}" removido`);
      confirmDeleteOpen = false;
      deleteTarget = null;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      deleteBusy = false;
    }
  }

  function fmtTime(secs: number): string {
    if (!secs || secs <= 0) return "—";
    return new Date(secs * 1000).toLocaleDateString();
  }

  onMount(load);
</script>

<section class="study-page">
  <PageHero
    title="Presets de deck"
    subtitle={$t("study.anki.presets.subtitle")}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="toolbar">
    <a class="back-link" href="/study/anki/decks">← Voltar pra Decks</a>
  </div>

  <div class="create-row">
    <input
      type="text"
      class="name-input"
      placeholder="Nome do novo preset…"
      bind:value={createName}
      onkeydown={(e) => { if (e.key === "Enter") create(); }}
    />
    <button
      type="button"
      class="btn primary"
      onclick={create}
      disabled={creating || !createName.trim()}
    >
      {creating ? "Criando…" : "Criar preset"}
    </button>
  </div>

  {#if loading}
    <div class="state">Carregando…</div>
  {:else if error}
    <div class="state err">{error}</div>
  {:else if summaries.length === 0}
    <div class="empty">
      <p>Nenhum preset ainda.</p>
    </div>
  {:else}
    <ul class="preset-list">
      {#each summaries as p (p.id)}
        <li class="preset-row">
          <div class="preset-info">
            <div class="preset-name">
              {p.name}
              {#if p.id === DEFAULT_PRESET_ID}
                <span class="badge">padrão</span>
              {/if}
            </div>
            <div class="preset-meta">
              {p.use_count === 1 ? "1 deck usa" : `${p.use_count} decks usam`}
              · atualizado {fmtTime(p.mtime_secs)}
            </div>
          </div>
          <div class="preset-actions">
            <button
              type="button"
              class="btn ghost sm"
              onclick={() => startEdit(p)}
            >
              Editar
            </button>
            <button
              type="button"
              class="btn ghost sm danger"
              onclick={() => askDelete(p)}
              disabled={p.id === DEFAULT_PRESET_ID || p.use_count > 0}
              title={p.id === DEFAULT_PRESET_ID
                ? $t("study.anki.presets.default_undeletable")
                : p.use_count > 0
                  ? "Mover decks pra outro preset antes de apagar"
                  : ""}
            >
              Apagar
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if editing && editForm}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) editing = null; }}
  >
    <div class="modal modal-wide" role="dialog" aria-modal="true">
      <h3>Editar preset · {editing.name}</h3>

      <div class="form-grid">
        <label class="field">
          <span>Novos cards / dia</span>
          <input
            type="number"
            min="0"
            bind:value={editForm.new_per_day}
          />
        </label>
        <label class="field">
          <span>Reviews / dia</span>
          <input
            type="number"
            min="0"
            bind:value={editForm.reviews_per_day}
          />
        </label>
        <label class="field">
          <span>Retenção desejada (FSRS)</span>
          <input
            type="number"
            step="0.01"
            min="0.7"
            max="0.99"
            bind:value={editForm.desired_retention}
          />
        </label>
        <label class="field">
          <span>Limite de leech</span>
          <input
            type="number"
            min="1"
            bind:value={editForm.leech_threshold}
          />
        </label>
        <label class="field">
          <span>Learning steps (min)</span>
          <input
            type="text"
            bind:value={editLearnRaw}
            placeholder="1 10"
          />
        </label>
        <label class="field">
          <span>Relearning steps (min)</span>
          <input
            type="text"
            bind:value={editRelearnRaw}
            placeholder="10"
          />
        </label>
      </div>

      <fieldset class="check-group">
        <legend>Comportamento</legend>
        <label class="check">
          <input type="checkbox" bind:checked={editForm.bury_new} />
          <span>Enterrar novos cards relacionados</span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={editForm.bury_reviews} />
          <span>Enterrar reviews relacionados</span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={editForm.disable_autoplay} />
          <span>Desativar autoplay de mídia</span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={editForm.show_timer} />
          <span>Mostrar timer durante estudo</span>
        </label>
      </fieldset>

      <div class="modal-foot">
        <button
          type="button"
          class="btn ghost"
          onclick={() => { editing = null; editForm = null; }}
          disabled={editBusy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={saveEdit}
          disabled={editBusy}
        >
          {editBusy ? "Salvando…" : "Salvar"}
        </button>
      </div>
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={confirmDeleteOpen}
  title="Apagar preset"
  message={deleteTarget
    ? `Apagar o preset "${deleteTarget.name}"? Decks que usam vão pro preset padrão.`
    : ""}
  confirmLabel="Apagar"
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
  }
  .back-link {
    color: var(--accent);
    font-size: 13px;
    text-decoration: none;
  }
  .back-link:hover {
    text-decoration: underline;
  }

  .create-row {
    display: flex;
    gap: 8px;
  }
  .name-input {
    flex: 1;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .name-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .state {
    padding: 24px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .state.err { color: var(--error, var(--accent)); }

  .empty {
    padding: 48px 16px;
    text-align: center;
    color: var(--tertiary);
    font-size: 13px;
  }

  .preset-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .preset-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
  }
  .preset-info {
    flex: 1;
    min-width: 0;
  }
  .preset-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text);
  }
  .badge {
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 10px;
    font-weight: 500;
  }
  .preset-meta {
    color: var(--tertiary);
    font-size: 12px;
    margin-top: 2px;
  }
  .preset-actions {
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
    transition: background 120ms ease;
  }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
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
  .btn.sm { padding: 5px 10px; font-size: 12px; }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
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
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .modal-wide { max-width: 720px; }
  .modal h3 { margin: 0; font-size: 15px; font-weight: 600; }

  .form-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .field input {
    padding: 7px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .field input:focus { outline: none; border-color: var(--accent); }

  .check-group {
    border: none;
    border-radius: var(--border-radius);
    padding: 10px 14px 12px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .check-group legend {
    padding: 0 6px;
    font-size: 11px;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--secondary);
    font-size: 13px;
    cursor: pointer;
  }
  .check input { accent-color: var(--accent); }

  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    border-top: none;
    padding-top: 12px;
  }
</style>
