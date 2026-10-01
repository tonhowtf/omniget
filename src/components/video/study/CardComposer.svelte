<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtTime } from "$lib/video/format";
  import { player, saveCard } from "$lib/video/player.svelte";
  import VIcon from "$components/video/player/VIcon.svelte";

  let saving = $state(false);
  let frontEl: HTMLTextAreaElement | undefined = $state();

  $effect(() => {
    if (player.cardDraft && frontEl) frontEl.focus();
  });

  async function save() {
    if (saving) return;
    saving = true;
    try {
      await saveCard();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }

  function keys(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Escape") player.cardDraft = null;
    else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    }
  }
</script>

{#if player.cardDraft}
  {@const d = player.cardDraft}
  <div class="scrim" role="presentation" onclick={() => (player.cardDraft = null)}>
    <div class="card" role="dialog" aria-modal="true" aria-label={$t("video.study.new_card") as string} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={keys}>
      <form
        class="inner"
        onsubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <div class="head">
          <VIcon name="cards" size={16} />
          <strong>{$t("video.study.new_card")}</strong>
          {#if d.t_ms != null}<span class="ts">{fmtTime(d.t_ms)}</span>{/if}
        </div>
        <label>
          <span>{$t("video.study.front")}</span>
          <textarea bind:this={frontEl} rows="3" value={d.front} oninput={(e) => player.cardDraft && (player.cardDraft.front = e.currentTarget.value)}></textarea>
        </label>
        <label>
          <span>{$t("video.study.back")}</span>
          <textarea
            rows="3"
            value={d.back}
            placeholder={$t("video.study.back_placeholder") as string}
            oninput={(e) => player.cardDraft && (player.cardDraft.back = e.currentTarget.value)}
          ></textarea>
        </label>
        <div class="acts">
          <button type="button" class="btn btn-sm" onclick={() => (player.cardDraft = null)}>{$t("video.cancel")}</button>
          <button type="submit" class="btn btn-sm btn-primary" disabled={saving || !d.front.trim()}>{$t("video.study.add_card")}</button>
        </div>
      </form>
      </div>
    </div>
  {/if}

<style>
  .scrim {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--pl-shade, var(--bg)) 40%, transparent);
    z-index: 45;
  }
  .card {
    width: min(460px, 92%);
    padding: 16px;
    border-radius: var(--radius-2xl);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-3);
    color: var(--text);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .head strong {
    flex: 1;
  }
  .ts {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--accent-hi);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  textarea {
    resize: vertical;
    padding: 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
  }
  textarea:focus {
    outline: none;
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .acts {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
