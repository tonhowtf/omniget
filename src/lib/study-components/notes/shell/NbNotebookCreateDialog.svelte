<script lang="ts">
  import { untrack } from "svelte";
  import {
    NOTEBOOK_LUCIDE_OPTIONS,
    NOTEBOOK_COLOR_SWATCHES,
  } from "$lib/study-notes/notebooks-store.svelte";

  type Props = {
    open: boolean;
    initialName?: string;
    initialColor?: string | null;
    initialIcon?: string | null;
    title?: string;
    confirmLabel?: string;
    onConfirm: (args: {
      name: string;
      color: string | null;
      iconLucide: string | null;
    }) => void;
    onClose: () => void;
  };

  // svelte-ignore state_referenced_locally
  let {
    open,
    initialName = "",
    initialColor = null,
    initialIcon = "book",
    title = "Novo notebook",
    confirmLabel = "Criar",
    onConfirm,
    onClose,
  }: Props = $props();

  let name = $state(initialName);
  let icon = $state<string | null>(initialIcon);
  let color = $state<string | null>(initialColor);
  let inputRef = $state<HTMLInputElement | null>(null);

  function snapshotProps() {
    return {
      name: initialName,
      icon: initialIcon,
      color: initialColor,
    };
  }

  $effect(() => {
    if (open) {
      const snap = untrack(snapshotProps);
      name = snap.name;
      icon = snap.icon;
      color = snap.color;
      queueMicrotask(() => inputRef?.focus());
    }
  });

  function submit() {
    const trimmed = name.trim();
    if (!trimmed) return;
    onConfirm({ name: trimmed, color, iconLucide: icon });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      submit();
    }
  }
</script>

{#if open}
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
      aria-label={title}
      data-modal="true"
    >
      <header>
        <h3>{title}</h3>
        <button
          type="button"
          class="x"
          onclick={onClose}
          aria-label="Fechar"
        >×</button>
      </header>

      <label class="field">
        <span>Nome</span>
        <input
          bind:this={inputRef}
          bind:value={name}
          type="text"
          maxlength="60"
          placeholder="Pessoal, Trabalho, Estudos…"
        />
      </label>

      <div class="field">
        <span>Ícone</span>
        <div class="grid icons">
          {#each NOTEBOOK_LUCIDE_OPTIONS as opt (opt)}
            <button
              type="button"
              class="icon-btn"
              class:active={icon === opt}
              title={opt}
              aria-pressed={icon === opt}
              onclick={() => (icon = opt)}
            >
              <span class="icon-name">{opt}</span>
            </button>
          {/each}
        </div>
      </div>

      <div class="field">
        <span>Cor</span>
        <div class="grid swatches">
          <button
            type="button"
            class="swatch none"
            class:active={color === null}
            aria-pressed={color === null}
            title="Sem cor"
            onclick={() => (color = null)}
          >∅</button>
          {#each NOTEBOOK_COLOR_SWATCHES as sw (sw)}
            <button
              type="button"
              class="swatch"
              class:active={color === sw}
              aria-pressed={color === sw}
              style:background={sw}
              title={sw}
              onclick={() => (color = sw)}
            ></button>
          {/each}
        </div>
      </div>

      <footer>
        <button type="button" class="btn ghost" onclick={onClose}>
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          disabled={!name.trim()}
          onclick={submit}
        >
          {confirmLabel}
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
    background: var(--bg, var(--page-bg));
    color: var(--text);
    border: none;
    border-radius: 14px;
    padding: 18px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
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
  .x:hover {
    color: var(--text);
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
  .grid {
    display: grid;
    gap: 6px;
  }
  .grid.icons {
    grid-template-columns: repeat(6, 1fr);
  }
  .grid.swatches {
    grid-template-columns: repeat(9, 1fr);
  }
  .icon-btn {
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 9.5px;
    padding: 6px 4px;
    cursor: pointer;
    transition: background 120ms, border-color 120ms;
    min-height: 40px;
    display: grid;
    place-items: center;
  }
  .icon-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .icon-btn.active {
    background: color-mix(in oklab, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--text);
  }
  .icon-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 100%;
    text-align: center;
  }
  .swatch {
    aspect-ratio: 1 / 1;
    border-radius: 999px;
    border: none;
    background: transparent;
    cursor: pointer;
    transition: transform 120ms, border-color 120ms;
  }
  .swatch:hover {
    transform: scale(1.08);
  }
  .swatch.active {
    border-color: var(--text);
    box-shadow: 0 0 0 2px color-mix(in oklab, var(--text) 30%, transparent);
  }
  .swatch.none {
    color: var(--tertiary);
    font-size: 14px;
    display: grid;
    place-items: center;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
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
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent);
  }
  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .btn.primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
