<script lang="ts">
  import { t } from "$lib/i18n";

  type Props = {
    open: boolean;
    onConfirm: (name: string) => void;
    onClose: () => void;
  };

  let { open, onConfirm, onClose }: Props = $props();
  let value = $state("");

  $effect(() => {
    if (!open) value = "";
  });

  function submit() {
    const trimmed = value.trim();
    if (!trimmed) return;
    onConfirm(trimmed);
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
    <div class="modal">
      <h3>{$t("study.library.notes_new_page")}</h3>
      <input
        type="text"
        placeholder={$t("study.notes.nb.page_name_placeholder")}
        bind:value
        onkeydown={(e) => {
          if (e.key === "Enter") submit();
          else if (e.key === "Escape") onClose();
        }}
      />
      <footer>
        <button class="btn ghost" onclick={onClose}>
          Cancelar
        </button>
        <button class="btn primary" onclick={submit}>Criar</button>
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
    max-width: 460px;
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
  .modal input {
    padding: 10px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 14px;
  }
  .modal input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .modal footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms ease, border-color 150ms ease;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  @media (prefers-reduced-motion: reduce) {
    .btn {
      transition: none;
    }
  }
</style>
