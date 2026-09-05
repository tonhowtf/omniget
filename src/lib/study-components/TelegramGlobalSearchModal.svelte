<script lang="ts">
  import { telegramSearchGlobalHits, type TelegramGlobalSearchHit } from "$lib/study-telegram-bridge";
  import { t } from "$lib/i18n";

  let {
    open = $bindable(false),
    onPickResult,
  } = $props<{
    open: boolean;
    onPickResult?: (hit: TelegramGlobalSearchHit) => void;
  }>();

  let query = $state("");
  let inputRef: HTMLInputElement | null = $state(null);
  let results = $state<TelegramGlobalSearchHit[]>([]);
  let loading = $state(false);
  let error = $state("");
  let debounce: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (open) {
      query = "";
      results = [];
      error = "";
      setTimeout(() => inputRef?.focus(), 50);
    }
  });

  function close() {
    open = false;
  }

  function onInput() {
    if (debounce) clearTimeout(debounce);
    if (!query.trim()) {
      results = [];
      loading = false;
      return;
    }
    debounce = setTimeout(runSearch, 350);
  }

  async function runSearch() {
    const q = query.trim();
    if (!q) return;
    loading = true;
    error = "";
    try {
      results = await telegramSearchGlobalHits({ query: q, limit: 50 });
    } catch (e: any) {
      error = typeof e === "string" ? e : (e?.message ?? $t("common.error"));
      results = [];
    } finally {
      loading = false;
    }
  }

  function pick(hit: TelegramGlobalSearchHit) {
    onPickResult?.(hit);
    close();
  }

  function fmtSize(n: number): string {
    if (n === 0) return "—";
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function mediaTypeIcon(type: string): string {
    switch (type) {
      case "photo": return "🖼️";
      case "video": return "🎬";
      case "audio": return "🎵";
      case "document": return "📄";
      case "webpage": return "🔗";
      default: return "📎";
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <div class="modal" role="dialog" aria-modal="true" aria-label={$t("telegram.global_search")} tabindex="-1">
      <header class="modal-header">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="search-icon">
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <input
          type="text"
          class="search-input"
          placeholder={$t("telegram.toolbox.global_search_placeholder")}
          aria-label={$t("telegram.global_search")}
          bind:value={query}
          bind:this={inputRef}
          oninput={onInput}
        />
        <kbd class="kbd">Esc</kbd>
      </header>

      <div class="results-area">
        {#if loading}
          <div class="status" role="status"><span class="spinner" aria-hidden="true"></span>{$t("common.loading")}</div>
        {:else if error}
          <div class="status status-error" role="alert">{error}</div>
        {:else if !query.trim()}
          <div class="status">{$t("telegram.toolbox.global_search_prompt")}</div>
        {:else if results.length === 0}
          <div class="status">{$t("telegram.toolbox.no_results_for", { query })}</div>
        {:else}
          <ul class="results-list">
            {#each results as hit (hit.chat_id + ":" + hit.message_id)}
              <li>
                <button type="button" class="hit" onclick={() => pick(hit)}>
                  <span class="hit-icon">{mediaTypeIcon(hit.media_type)}</span>
                  <div class="hit-info">
                    <span class="hit-title">{hit.file_name}</span>
                    <span class="hit-meta">
                      {hit.chat_title}
                      <span class="hit-dot">·</span>
                      {fmtSize(hit.file_size)}
                    </span>
                  </div>
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" class="hit-arrow">
                    <path d="M9 6l6 6-6 6" />
                  </svg>
                </button>
              </li>
            {/each}
          </ul>
          <div class="results-footer">
            <span>{$t("telegram.toolbox.results_count", { count: results.length })}</span>
            <span class="kbd-hint">{$t("telegram.toolbox.search_keyboard_hint")}</span>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 180;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 10vh;
    animation: fade-in 120ms ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .modal {
    width: min(680px, 92vw);
    max-height: 75vh;
    background: var(--surface, var(--button));
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.35);
    overflow: hidden;
  }

  .modal-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 18px;
    border-bottom: 1px solid var(--input-border);
  }

  .search-icon {
    color: var(--gray);
    flex-shrink: 0;
  }

  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--secondary);
    font-family: inherit;
    font-size: 15px;
    outline: none;
  }

  .search-input::placeholder {
    color: var(--gray);
  }

  .kbd {
    font-family: monospace;
    font-size: 10.5px;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--button-elevated);
    color: var(--gray);
    border: 1px solid var(--input-border);
    flex-shrink: 0;
  }

  .results-area {
    flex: 1;
    overflow-y: auto;
    min-height: 60px;
  }

  .status {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px 16px;
    color: var(--gray);
    font-size: 13px;
  }

  .status-error {
    color: var(--red);
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .results-list {
    list-style: none;
    margin: 0;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .hit {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 120ms;
  }

  .hit:hover {
    background: color-mix(in oklab, var(--accent, var(--blue)) 12%, transparent);
  }

  .hit-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    font-size: 16px;
    flex-shrink: 0;
  }

  .hit-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .hit-title {
    font-size: 13.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hit-meta {
    font-size: 11.5px;
    color: var(--gray);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hit-dot {
    margin: 0 4px;
  }

  .hit-arrow {
    color: var(--gray);
    flex-shrink: 0;
  }

  .results-footer {
    display: flex;
    justify-content: space-between;
    padding: 8px 18px;
    border-top: 1px solid var(--input-border);
    font-size: 11px;
    color: var(--gray);
  }

  .kbd-hint {
    font-family: monospace;
  }
</style>
