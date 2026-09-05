<script lang="ts">
  type TransferRecord = {
    id: number;
    fileName: string;
    sizeBytes?: number;
    percent: number;
    status: "downloading" | "done" | "error";
    error?: string;
    completedAt?: number;
    path?: string;
  };

  let {
    open = $bindable(false),
    active,
    history,
    onClearHistory,
  } = $props<{
    open: boolean;
    active: TransferRecord[];
    history: TransferRecord[];
    onClearHistory?: () => void;
  }>();

  function close() {
    open = false;
  }

  function fmtSize(n: number | undefined): string {
    if (!n) return "—";
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function fmtTime(ts: number | undefined): string {
    if (!ts) return "";
    const d = new Date(ts);
    return d.toLocaleString();
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) close(); }}
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
  >
    <aside class="drawer" role="dialog" aria-modal="true" aria-label="Transferências">
      <header class="drawer-header">
        <div>
          <h2>Transferências</h2>
          <p class="subtitle">{active.length} ativa{active.length === 1 ? "" : "s"} · {history.length} no histórico</p>
        </div>
        <button type="button" class="icon-btn" onclick={close} aria-label="Fechar">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18" />
            <path d="M6 6l12 12" />
          </svg>
        </button>
      </header>

      <div class="drawer-body">
        <section>
          <span class="section-label">Em andamento</span>
          {#if active.length === 0}
            <p class="empty-text">Nenhum download ativo.</p>
          {:else}
            <ul class="transfer-list">
              {#each active as t (t.id)}
                <li class="transfer-item">
                  <div class="transfer-info">
                    <span class="transfer-name">{t.fileName}</span>
                    <span class="transfer-meta">{fmtSize(t.sizeBytes)} · {Math.round(t.percent)}%</span>
                  </div>
                  <div class="progress-outer">
                    <div class="progress-inner" style:width="{Math.max(2, Math.min(100, t.percent))}%"></div>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <section>
          <div class="section-row">
            <span class="section-label">Histórico</span>
            {#if history.length > 0 && onClearHistory}
              <button type="button" class="ghost-btn" onclick={onClearHistory}>Limpar</button>
            {/if}
          </div>
          {#if history.length === 0}
            <p class="empty-text">Sem transferências recentes.</p>
          {:else}
            <ul class="transfer-list">
              {#each history as t (t.id)}
                <li class="transfer-item history-item">
                  <span class="status-dot" class:status-error={t.status === "error"} class:status-done={t.status === "done"}></span>
                  <div class="transfer-info">
                    <span class="transfer-name">{t.fileName}</span>
                    <span class="transfer-meta">
                      {t.status === "done" ? "Concluído" : "Erro"}
                      · {fmtSize(t.sizeBytes)}
                      {#if t.completedAt}· {fmtTime(t.completedAt)}{/if}
                    </span>
                    {#if t.error}<span class="transfer-error">{t.error}</span>{/if}
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>
    </aside>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 100;
    display: flex;
    justify-content: flex-end;
    animation: fade-in 150ms ease;
  }

  .drawer {
    width: min(440px, 100vw);
    height: 100%;
    background: var(--surface, var(--button));
    display: flex;
    flex-direction: column;
    animation: slide-in 200ms cubic-bezier(0.22, 1, 0.36, 1);
    box-shadow: -2px 0 12px rgba(0, 0, 0, 0.2);
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes slide-in {
    from { transform: translateX(20px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .drawer-header {
    display: flex;
    align-items: flex-start;
    gap: var(--padding);
    padding: var(--padding);
    border-bottom: none;
  }

  .drawer-header > div {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .drawer-header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--secondary);
  }

  .subtitle {
    margin: 0;
    font-size: 11.5px;
    color: var(--gray);
  }

  .icon-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--border-radius);
  }

  .icon-btn:hover {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--padding);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .section-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 6px;
  }

  .ghost-btn {
    background: transparent;
    border: none;
    color: var(--gray);
    font-family: inherit;
    font-size: 11px;
    padding: 3px 10px;
    border-radius: var(--border-radius);
    cursor: pointer;
  }

  .ghost-btn:hover {
    color: var(--secondary);
    background: var(--button-elevated);
  }

  .empty-text {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--gray);
    text-align: center;
    padding: var(--padding) 0;
  }

  .transfer-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .transfer-item {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .history-item {
    flex-direction: row;
    align-items: flex-start;
    gap: 10px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--gray);
    margin-top: 6px;
    flex-shrink: 0;
  }

  .status-dot.status-done {
    background: var(--green);
  }

  .status-dot.status-error {
    background: var(--red);
  }

  .transfer-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .transfer-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .transfer-meta {
    font-size: 11px;
    color: var(--gray);
  }

  .transfer-error {
    font-size: 11px;
    color: var(--red);
    margin-top: 2px;
  }

  .progress-outer {
    width: 100%;
    height: 4px;
    background: var(--button-elevated);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-inner {
    height: 100%;
    background: var(--blue);
    border-radius: 2px;
    transition: width 200ms;
  }
</style>
