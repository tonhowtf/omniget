<script lang="ts">
  import { t } from "$lib/i18n";
  import AccountCard from "./AccountCard.svelte";
  import {
    orderByChain,
    reorder,
    type AccountView,
    type SandboxMode,
  } from "$lib/stores/llm-accounts-store.svelte";

  interface Props {
    accounts: AccountView[];
    chain: string[];
    available: boolean;
    nowMs?: number;
    busy?: boolean;
    onreorder: (chain: string[]) => void;
    onactivate: (id: string) => void;
    onlogin: (id: string) => void;
    ontoggle: (id: string, disabled: boolean) => void;
    onremove: (id: string, purge: boolean) => void;
    onsandbox: (id: string, sandbox: SandboxMode) => void;
  }

  let {
    accounts,
    chain,
    available,
    nowMs = Date.now(),
    busy = false,
    onreorder,
    onactivate,
    onlogin,
    ontoggle,
    onremove,
    onsandbox,
  }: Props = $props();

  let ordered = $derived(orderByChain(accounts, chain));
  let dragging = $state<string | null>(null);
  let over = $state<string | null>(null);

  function move(id: string, delta: number) {
    const from = chain.indexOf(id);
    if (from < 0) return;
    onreorder(reorder(chain, id, from + delta));
  }

  function onDrop(targetId: string) {
    const id = dragging;
    dragging = null;
    over = null;
    if (!id || id === targetId) return;
    onreorder(reorder(chain, id, chain.indexOf(targetId)));
  }
</script>

<ol class="chain">
  {#each ordered as account, i (account.id)}
    <li
      class="row"
      class:over={over === account.id && dragging !== account.id}
      ondragover={(e) => {
        e.preventDefault();
        over = account.id;
      }}
      ondragleave={() => {
        if (over === account.id) over = null;
      }}
      ondrop={(e) => {
        e.preventDefault();
        onDrop(account.id);
      }}
    >
      <div class="handle-col">
        <div
          class="handle"
          role="button"
          tabindex="-1"
          draggable="true"
          aria-hidden="true"
          ondragstart={() => (dragging = account.id)}
          ondragend={() => {
            dragging = null;
            over = null;
          }}
        >
          ⠿
        </div>
        <button
          class="move"
          type="button"
          disabled={busy || i === 0}
          aria-label="{$t('llm.accounts.move_up')} — {account.label}"
          onclick={() => move(account.id, -1)}
        >
          ↑
        </button>
        <button
          class="move"
          type="button"
          disabled={busy || i === ordered.length - 1}
          aria-label="{$t('llm.accounts.move_down')} — {account.label}"
          onclick={() => move(account.id, 1)}
        >
          ↓
        </button>
      </div>
      <AccountCard
        {account}
        rank={i + 1}
        {available}
        {nowMs}
        {busy}
        {onactivate}
        {onlogin}
        {ontoggle}
        {onremove}
        {onsandbox}
      />
    </li>
  {/each}
</ol>

<style>
  .chain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .row {
    display: grid;
    grid-template-columns: 26px 1fr;
    gap: var(--space-2);
    align-items: start;
    border-radius: var(--radius-md);
  }
  .row.over {
    outline: 2px dashed var(--accent);
    outline-offset: 3px;
  }
  .handle-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding-top: var(--space-4);
  }
  .handle {
    cursor: grab;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1;
    padding: 2px;
  }
  .move {
    width: 22px;
    height: 20px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-size: var(--text-caption);
  }
  .move:disabled {
    opacity: 0.3;
    cursor: default;
  }
  @media (hover: hover) {
    .move:not(:disabled):hover {
      background: var(--surface-hi);
      color: var(--text);
    }
  }
</style>
