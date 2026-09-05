<script lang="ts">
  import { notebooksStore } from "$lib/study-notes/notebooks-store.svelte";

  type Props = {
    open: boolean;
    notebookId: number | null;
    onClose: () => void;
  };

  let { open, notebookId, onClose }: Props = $props();

  let assetIdDraft = $state<string>("");
  let busy = $state(false);
  let error = $state<string>("");

  const current = $derived(
    notebookId == null ? null : notebooksStore.byId(notebookId),
  );

  $effect(() => {
    if (open) {
      assetIdDraft = current?.cover_asset_id?.toString() ?? "";
      error = "";
    }
  });

  async function apply() {
    if (notebookId == null) return;
    const trimmed = assetIdDraft.trim();
    let asset: number | null = null;
    if (trimmed.length > 0) {
      const n = Number.parseInt(trimmed, 10);
      if (!Number.isFinite(n) || n <= 0) {
        error = "ID precisa ser inteiro positivo";
        return;
      }
      asset = n;
    }
    busy = true;
    error = "";
    try {
      await notebooksStore.setCover(notebookId, asset);
      onClose();
    } catch (e) {
      error = String(e ?? "falha ao aplicar capa");
    } finally {
      busy = false;
    }
  }

  async function clear() {
    if (notebookId == null) return;
    busy = true;
    try {
      await notebooksStore.setCover(notebookId, null);
      onClose();
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void apply();
    }
  }
</script>

{#if open && notebookId != null}
  <div
    class="backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
    onkeydown={onKey}
  >
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Capa do notebook"
      data-modal="true"
    >
      <header>
        <h3>Capa de {current?.name ?? "notebook"}</h3>
        <button type="button" class="x" onclick={onClose} aria-label="Fechar">×</button>
      </header>

      <p class="hint">
        Use o ID de um asset já registrado em <code>note_assets</code>.
        Para fazer upload de uma nova imagem, use o gerenciador de capas
        de uma página primeiro e depois aponte o ID aqui.
      </p>

      <label class="field">
        <span>Asset ID</span>
        <input
          type="text"
          inputmode="numeric"
          placeholder="ex: 42"
          bind:value={assetIdDraft}
        />
      </label>

      {#if error}
        <p class="error">{error}</p>
      {/if}

      <footer>
        <button
          type="button"
          class="btn ghost"
          onclick={clear}
          disabled={busy || current?.cover_asset_id == null}
        >
          Remover capa
        </button>
        <span class="spacer"></span>
        <button type="button" class="btn ghost" onclick={onClose} disabled={busy}>
          Cancelar
        </button>
        <button type="button" class="btn primary" onclick={apply} disabled={busy}>
          Aplicar
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: grid;
    place-items: center;
    background: color-mix(in oklab, black 40%, transparent);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }
  .dialog {
    width: min(440px, 92vw);
    background: var(--bg);
    color: var(--text);
    border: none;
    border-radius: 14px;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.4);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  header h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .x {
    background: transparent;
    border: 0;
    color: var(--tertiary);
    font-size: 18px;
    cursor: pointer;
    padding: 0 4px;
  }
  .hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
    line-height: 1.45;
  }
  .hint code {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    font-size: 11px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field > span {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tertiary);
  }
  input[type="text"] {
    padding: 8px 10px;
    border: none;
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  input[type="text"]:focus {
    outline: none;
    border-color: var(--accent);
  }
  .error {
    margin: 0;
    color: #ef4444;
    font-size: 12px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .btn {
    padding: 7px 14px;
    border-radius: 8px;
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms, border-color 150ms;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent);
  }
  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
</style>
