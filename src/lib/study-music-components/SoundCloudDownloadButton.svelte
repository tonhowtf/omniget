<script lang="ts">
  import { downloadStore } from "$lib/study-music/download-store.svelte";
  import type { ScTrack } from "$lib/study-music/soundcloud-store.svelte";

  type Props = {
    track: ScTrack;
    onTrigger: (track: ScTrack, advanced: boolean) => void;
  };

  let { track, onTrigger }: Props = $props();

  const job = $derived(downloadStore.getJobByTrackId(track.id));
  const busy = $derived(
    !!job && job.stage !== "done" && job.stage !== "skipped" && !job.error,
  );
  const pct = $derived(job?.progressPct ?? 0);
  const stage = $derived(job?.stage);
</script>

{#if busy}
  <button
    type="button"
    class="btn busy"
    disabled
    aria-label={`Baixando — ${pct}%`}
    title={`Baixando — ${pct}%`}
  >
    <span class="ring" style:--pct="{pct}">
      <span class="pct">{pct < 1 ? "" : pct}</span>
    </span>
  </button>
{:else if job && (job.stage === "done" || job.stage === "skipped") && !job.error}
  <button
    type="button"
    class="btn done"
    aria-label="Baixado"
    title="Baixado"
    onclick={(e) => { e.stopPropagation(); onTrigger(track, e.shiftKey); }}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="20 6 9 17 4 12"/>
    </svg>
  </button>
{:else}
  <button
    type="button"
    class="btn"
    onclick={(e) => { e.stopPropagation(); onTrigger(track, e.shiftKey); }}
    aria-label="Baixar"
    title={"Baixar (Shift = avançado)"}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
      <polyline points="7 10 12 15 17 10"/>
      <line x1="12" y1="15" x2="12" y2="3"/>
    </svg>
  </button>
{/if}

<style>
  .btn {
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: 0;
    color: rgba(255, 255, 255, 0.5);
    cursor: pointer;
    display: grid;
    place-items: center;
    border-radius: 4px;
    transition: background 200ms ease, color 200ms ease;
  }
  .btn:hover { background: rgba(255, 255, 255, 0.1); color: #ff5500; }
  .btn:disabled { cursor: default; }
  .btn.busy:hover { background: transparent; }
  .btn.done { color: #4ade80; }
  .btn.done:hover { color: #4ade80; background: rgba(74, 222, 128, 0.12); }
  .ring {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background:
      conic-gradient(#ff5500 calc(var(--pct, 0) * 1%), rgba(255, 255, 255, 0.16) 0);
    display: grid;
    place-items: center;
    position: relative;
  }
  .ring::after {
    content: "";
    position: absolute;
    inset: 3px;
    background: var(--button);
    border-radius: 50%;
  }
  .pct {
    position: relative;
    z-index: 1;
    font-size: 8px;
    font-weight: 700;
    color: #ff5500;
    line-height: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .btn { transition: none; }
  }
</style>
