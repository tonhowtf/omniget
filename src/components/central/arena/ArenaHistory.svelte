<script lang="ts">
  // Lista lateral das arenas (mais novas primeiro) com o estado e o vencedor.
  import { t } from "$lib/i18n";
  import { driverTint } from "$lib/central/threads-ui/drivers";
  import { isActive, type ArenaView } from "$lib/central/arena";
  import Icon from "$components/central/threads/Icon.svelte";

  let {
    arenas,
    selected,
    onselect,
  }: { arenas: ArenaView[]; selected: string | null; onselect: (id: string) => void } = $props();

  function when(iso: string): string {
    const d = new Date(iso);
    const today = new Date();
    return d.toDateString() === today.toDateString()
      ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      : d.toLocaleDateString([], { month: "short", day: "numeric" });
  }
</script>

<ul class="list">
  {#each arenas as a (a.arenaId)}
    {@const winner = a.entries.find((e) => e.winner)}
    {@const live = a.entries.some(isActive)}
    <li>
      <button type="button" class="item" class:sel={a.arenaId === selected} onclick={() => onselect(a.arenaId)} aria-current={a.arenaId === selected ? "true" : undefined}>
        <div class="line">
          <span class="title">{a.title}</span>
          <span class="when">{when(a.createdAt)}</span>
        </div>
        <div class="line sub">
          <span class="dots">
            {#each a.entries as e (e.entryId)}
              <span class="dot" class:win={e.winner} class:fail={e.status === "failed"} style:--tint={driverTint(e.driver)} title={e.label}></span>
            {/each}
          </span>
          <span class="state st-{a.status}">
            {#if live}<Icon name="circle-notch" size={11} />{/if}
            {#if a.status === "applied" && winner}
              {winner.label}
            {:else}
              {$t(`llm.central.arena.arena_status.${a.status}`)}
            {/if}
          </span>
        </div>
      </button>
    </li>
  {:else}
    <li class="empty">{$t("llm.central.arena.history.empty")}</li>
  {/each}
</ul>

<style>
  .list { list-style: none; margin: 0; padding: 4px; display: flex; flex-direction: column; gap: 2px; }
  .item { width: 100%; border: 0; background: transparent; text-align: left; color: var(--text); padding: 7px 9px; border-radius: var(--radius-md); cursor: pointer; display: flex; flex-direction: column; gap: 3px; }
  .item:hover { background: var(--fill-1); }
  .item.sel { background: var(--accent-soft, var(--fill-2)); }
  .line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .title { flex: 1; min-width: 0; font-size: var(--text-base); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { font-size: var(--text-xs); color: var(--text-muted); flex-shrink: 0; }
  .sub { justify-content: space-between; }
  .dots { display: inline-flex; gap: 3px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--tint); opacity: 0.8; }
  .dot.win { box-shadow: 0 0 0 2px color-mix(in srgb, var(--success) 70%, transparent); opacity: 1; }
  .dot.fail { opacity: 0.3; }
  .state { font-size: var(--text-xs); color: var(--text-muted); display: inline-flex; align-items: center; gap: 3px; max-width: 60%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .st-applied { color: var(--success); }
  .st-running { color: var(--accent-text, var(--accent)); }
  .empty { color: var(--text-muted); font-size: var(--text-sm); padding: 12px; }
</style>
