<script lang="ts">
  import { musicPlayer } from "$lib/study-music/player-store.svelte";
  import { t } from "$lib/i18n";

  type Props = {
    open: boolean;
    onClose: () => void;
  };

  let { open, onClose }: Props = $props();

  const PRESETS_MIN = [5, 15, 30, 45, 60, 90, 120];

  let customMinutes = $state(20);
  let now = $state(Date.now());

  $effect(() => {
    if (!open) return;
    const tick = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(tick);
  });

  const remainingSec = $derived.by(() => {
    const ends = musicPlayer.sleepTimerEndsAt;
    if (!ends) return 0;
    return Math.max(0, Math.floor((ends - now) / 1000));
  });

  const remainingLabel = $derived.by(() => {
    const s = remainingSec;
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
  });

  function applyPreset(minutes: number) {
    musicPlayer.setSleepTimerMinutes(minutes);
  }

  function applyCustom() {
    const m = Math.max(1, Math.min(720, Math.floor(customMinutes)));
    musicPlayer.setSleepTimerMinutes(m);
  }

  function endOfTrack() {
    musicPlayer.setSleepEndOfTrack(true);
  }

  function cancel() {
    musicPlayer.cancelSleepTimer();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
    onkeydown={onKey}
  >
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <header class="head">
        <h3>{$t("study.music.sleep_timer_title")}</h3>
        <button type="button" class="close" onclick={onClose} aria-label={$t("study.common.close") as string}>×</button>
      </header>

      {#if musicPlayer.sleepTimerEndsAt}
        <div class="active">
          <span class="active-label">{$t("study.music.sleep_timer_active")}</span>
          <span class="active-value">{remainingLabel}</span>
        </div>
      {:else if musicPlayer.sleepTimerEndOfTrack}
        <div class="active">
          <span class="active-label">{$t("study.music.sleep_timer_active")}</span>
          <span class="active-value">{$t("study.music.sleep_timer_end_of_track")}</span>
        </div>
      {/if}

      <div class="body">
        <h4>{$t("study.music.sleep_timer_presets")}</h4>
        <div class="presets">
          {#each PRESETS_MIN as min (min)}
            <button type="button" class="preset" onclick={() => applyPreset(min)}>
              {min} {$t("study.music.sleep_timer_minutes_short")}
            </button>
          {/each}
        </div>

        <h4>{$t("study.music.sleep_timer_custom")}</h4>
        <div class="custom-row">
          <input
            type="number"
            class="num"
            min="1"
            max="720"
            step="1"
            bind:value={customMinutes}
          />
          <span class="unit">{$t("study.music.sleep_timer_minutes")}</span>
          <button type="button" class="apply" onclick={applyCustom}>
            {$t("study.music.sleep_timer_apply")}
          </button>
        </div>

        <h4>{$t("study.music.sleep_timer_other")}</h4>
        <div class="row">
          <button type="button" class="alt" onclick={endOfTrack}>
            {$t("study.music.sleep_timer_end_of_track")}
          </button>
          <button type="button" class="alt danger" onclick={cancel}>
            {$t("study.music.sleep_timer_cancel")}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 999;
  }
  .dialog {
    background: var(--surface);
    border: none;
    border-radius: 12px;
    width: min(440px, 90vw);
    max-height: 80vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 18px;
    border-bottom: none;
  }
  .head h3 {
    margin: 0;
    font-size: 16px;
  }
  .close {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 22px;
    cursor: pointer;
    line-height: 1;
  }
  .active {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 18px;
    background: rgba(255, 213, 100, 0.08);
    border-bottom: 1px solid rgba(255, 213, 100, 0.18);
  }
  .active-label {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.7);
  }
  .active-value {
    font-family: ui-monospace, "Cascadia Code", monospace;
    font-size: 15px;
    color: #ffd564;
  }
  .body {
    padding: 14px 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .body h4 {
    margin: 8px 0 4px;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.5);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 500;
  }
  .presets {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .preset {
    appearance: none;
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: inherit;
    padding: 6px 12px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
  }
  .preset:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .custom-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .num {
    width: 80px;
    appearance: textfield;
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: inherit;
    padding: 6px 10px;
    border-radius: 8px;
    font: inherit;
    font-variant-numeric: tabular-nums;
  }
  .unit {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.6);
  }
  .apply {
    appearance: none;
    background: var(--accent);
    border: none;
    color: var(--on-accent);
    padding: 6px 14px;
    border-radius: 8px;
    cursor: pointer;
    font: inherit;
    font-weight: 500;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .alt {
    appearance: none;
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: inherit;
    padding: 8px 14px;
    border-radius: 8px;
    cursor: pointer;
    font: inherit;
  }
  .alt:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .alt.danger {
    color: #ff8b6f;
    border-color: rgba(255, 139, 111, 0.3);
  }
  .alt.danger:hover {
    background: rgba(255, 139, 111, 0.1);
  }
</style>
