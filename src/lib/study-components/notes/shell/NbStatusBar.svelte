<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { notesShell } from "$lib/study-notes/shell-store.svelte";
  import NbActiveNotebookBadge from "./NbActiveNotebookBadge.svelte";

  let now = $state(new Date());
  let timer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    timer = setInterval(() => {
      now = new Date();
    }, 30_000);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const clockText = $derived(
    `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`,
  );

  const modeLabel = $derived(
    notesShell.mode === "edit"
      ? "edit"
      : notesShell.mode === "source"
        ? "source"
        : "preview",
  );

  const pageLabel = $derived(notesShell.currentPageTitle ?? notesShell.currentPageName ?? "—");

  const wordsLabel = $derived(`${notesShell.wordCount.toLocaleString("pt-BR")} palavras`);
  const charsLabel = $derived(`${notesShell.charCount.toLocaleString("pt-BR")} chars`);
</script>

<div class="nb-status-bar" role="status" aria-live="polite">
  <span class="seg counts">
    <span class="metric">{charsLabel}</span>
    <span class="dot">·</span>
    <span class="metric">{wordsLabel}</span>
  </span>
  <span class="seg">
    <span class="ic" aria-hidden="true">✏</span>
    <span class="value">{modeLabel}</span>
  </span>
  <span class="seg notebook">
    <NbActiveNotebookBadge />
  </span>
  <span class="seg page">
    <span class="ic" aria-hidden="true">📄</span>
    <span class="value">{pageLabel}</span>
  </span>
  <span class="seg saving" class:on={notesShell.saving}>
    <span class="dot-mark" aria-hidden="true"></span>
    <span class="value">{notesShell.saving ? "Salvando…" : "Salvo"}</span>
  </span>
  <span class="spacer"></span>
  <span class="seg clock">
    <span class="value">{clockText}</span>
  </span>
</div>

<style>
  .nb-status-bar {
    height: 100%;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 14px;
    font-size: 11px;
    font-family: var(--font-body, inherit);
    color: var(--tertiary, var(--muted-fg));
    background: color-mix(in oklab, var(--surface-bg, var(--primary)) 70%, transparent);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    overflow: hidden;
    white-space: nowrap;
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    line-height: 1;
  }
  .seg.counts .metric {
    font-variant-numeric: tabular-nums;
  }
  .dot {
    opacity: 0.5;
  }
  .ic {
    font-size: 11px;
    line-height: 1;
    opacity: 0.7;
  }
  .value {
    color: var(--secondary, var(--text));
  }
  .seg.page .value {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg.saving .dot-mark {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--success) 80%, transparent);
    transition: background 200ms ease;
  }
  .seg.saving.on .dot-mark {
    background: var(--accent);
    animation: nb-pulse 1.2s ease-in-out infinite;
  }
  .spacer {
    flex: 1;
  }
  .seg.clock .value {
    font-variant-numeric: tabular-nums;
  }
  @keyframes nb-pulse {
    0%, 100% { opacity: 0.55; }
    50% { opacity: 1; }
  }
  @media (max-width: 760px) {
    .seg.notebook,
    .seg.page,
    .seg.clock {
      display: none;
    }
  }
</style>
