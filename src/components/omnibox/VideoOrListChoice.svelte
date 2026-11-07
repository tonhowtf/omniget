<script lang="ts">
  import { t } from "$lib/i18n";

  let {
    choice,
    isMix,
    onChoose,
  }: {
    choice: "video" | "list";
    isMix: boolean;
    onChoose: (next: "video" | "list") => void;
  } = $props();
</script>

<fieldset class="video-or-list">
  <legend>{$t(isMix ? "omnibox.video_or_list_mix" : "omnibox.video_or_list")}</legend>
  <div class="options">
    <button type="button" aria-pressed={choice === "video"} class:active={choice === "video"} onclick={() => onChoose("video")}>
      {$t("omnibox.video_or_list_video")}
    </button>
    <button type="button" aria-pressed={choice === "list"} class:active={choice === "list"} onclick={() => onChoose("list")}>
      {$t(isMix ? "omnibox.video_or_list_mix_list" : "omnibox.video_or_list_list")}
    </button>
  </div>
</fieldset>

<style>
  .video-or-list {
    border: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  legend {
    padding: 0;
    margin-block-end: var(--space-2);
    font-size: 13px;
    color: var(--text);
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  button {
    flex: 1 1 140px;
    min-height: 32px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }

  button.active {
    background: var(--accent);
    box-shadow: none;
    color: var(--on-accent, #fff);
  }

  button:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
    outline-offset: 2px;
  }
</style>
