<script lang="ts">
  import {
    notesCoverSetExternal,
    notesCoverRemove,
  } from "$lib/notes-bridge";

  type Props = {
    open: boolean;
    pageId: number | null;
    currentCoverUrl: string | null;
    onSaved: () => void;
    onClose: () => void;
  };

  let { open, pageId, currentCoverUrl, onSaved, onClose }: Props = $props();

  let urlDraft = $state("");
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    if (open) {
      urlDraft = currentCoverUrl ?? "";
      error = "";
    }
  });

  async function saveExternal() {
    if (!pageId) return;
    const url = urlDraft.trim();
    if (!url) return;
    if (!/^https?:\/\//i.test(url)) {
      error = "URL precisa começar com http:// ou https://";
      return;
    }
    busy = true;
    error = "";
    try {
      await notesCoverSetExternal({ pageId, url });
      onSaved();
      onClose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function removeCover() {
    if (!pageId) return;
    busy = true;
    error = "";
    try {
      await notesCoverRemove(pageId);
      onSaved();
      onClose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div class="modal" role="dialog" aria-label="Cover image">
      <h3>Capa da página</h3>

      <p class="hint">
        Cole uma URL externa (https://...). Upload local de arquivo entra em sessão futura.
      </p>

      <label class="field">
        <span>URL da imagem</span>
        <input
          type="url"
          bind:value={urlDraft}
          placeholder="https://exemplo.com/banner.jpg"
          disabled={busy}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void saveExternal();
            } else if (e.key === "Escape") {
              onClose();
            }
          }}
        />
      </label>

      {#if urlDraft && /^https?:\/\//i.test(urlDraft.trim())}
        <div class="preview">
          <img src={urlDraft.trim()} alt="Preview" loading="lazy" />
        </div>
      {/if}

      {#if error}
        <p class="error-msg">{error}</p>
      {/if}

      <footer>
        {#if currentCoverUrl}
          <button
            type="button"
            class="btn ghost danger"
            onclick={removeCover}
            disabled={busy}
          >
            Remover capa
          </button>
        {/if}
        <span class="footer-spacer"></span>
        <button type="button" class="btn ghost" onclick={onClose} disabled={busy}>
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={saveExternal}
          disabled={busy || !urlDraft.trim()}
        >
          {busy ? "Salvando…" : "Salvar"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, black 50%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 90;
    padding: var(--padding);
  }
  .modal {
    width: 100%;
    max-width: 520px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modal h3 {
    margin: 0;
    font-size: 16px;
  }
  .hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: 11px;
    color: var(--secondary);
  }
  .field input {
    padding: 8px 10px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .field input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .preview {
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    background: var(--bg);
    max-height: 200px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .preview img {
    max-width: 100%;
    max-height: 200px;
    object-fit: cover;
  }
  .error-msg {
    margin: 0;
    color: var(--error);
    font-size: 12px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .footer-spacer {
    flex: 1 1 auto;
  }
  .btn {
    padding: 6px 14px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:disabled {
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
  .btn.ghost.danger {
    color: var(--error, var(--accent));
    border-color: color-mix(in oklab, var(--error, var(--accent)) 40%, var(--input-border));
  }
  .btn.ghost.danger:hover:not(:disabled) {
    background: color-mix(in oklab, var(--error, var(--accent)) 10%, transparent);
  }
</style>
