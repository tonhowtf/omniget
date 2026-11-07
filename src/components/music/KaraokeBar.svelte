<script lang="ts">
  import { t } from "$lib/i18n";
  import { backToOriginal, karaoke, setVocalMix, singAlong, stemsFor } from "$lib/music/karaoke.svelte";
  import { player } from "$lib/music/player-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let cur = $derived(player.current);
  let hit = $derived.by(() => {
    void karaoke.sets;
    return stemsFor(cur?.id);
  });
  let pct = $derived(Math.round(karaoke.vocal * 100));
</script>

{#if cur && !player.radio}
  {#if hit?.role === "instrumental"}
    <section class="kb on" aria-label={$t("music.karaoke.title") as string}>
      <span class="badge"><MusicIcon name="lyrics" size={14} /> {$t("music.karaoke.title")}</span>
      <label class="mixer">
        <span>{$t("music.karaoke.vocal_guide")}</span>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={pct}
          oninput={(e) => setVocalMix(Number((e.currentTarget as HTMLInputElement).value) / 100)}
          aria-valuetext={pct ? `${pct}%` : ($t("music.karaoke.vocal_off") as string)}
        />
        <b>{pct ? `${pct}%` : $t("music.karaoke.vocal_off")}</b>
      </label>
      {#if hit.set.source_id != null}
        <button class="btn btn-sm btn-ghost" onclick={() => hit && void backToOriginal(hit.set)}>{$t("music.karaoke.back")}</button>
      {/if}
    </section>
  {:else if hit}
    <section class="kb">
      <span class="hint">{$t("music.karaoke.available")}</span>
      <button class="btn btn-sm btn-primary" onclick={() => hit && void singAlong(hit.set)}><MusicIcon name="lyrics" size={13} /> {$t("music.karaoke.sing")}</button>
    </section>
  {:else}
    <button class="btn btn-sm btn-ghost split" onclick={() => cur && go({ kind: "tools", tab: "stems", ids: [cur.id] })}>
      <MusicIcon name="split" size={13} /> {$t("music.karaoke.make")}
    </button>
  {/if}
{/if}

<style>
  .kb {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: 8px 12px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .kb.on {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 var(--hairline) var(--accent);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--accent-hi);
  }
  .mixer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 200px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .mixer input {
    flex: 1;
    accent-color: var(--accent);
  }
  .mixer b {
    min-width: 5ch;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text);
    font-weight: 600;
  }
  .hint {
    font-size: var(--text-sm);
    color: var(--text-dim);
    flex: 1;
  }
  .split {
    align-self: flex-start;
  }
</style>
