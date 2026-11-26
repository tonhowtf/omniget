<script lang="ts">
  import { t } from "$lib/i18n";
  import { player, setPanel, togglePanel, type PanelTab } from "$lib/video/player.svelte";
  import AskTab from "#components/video/study/AskTab.svelte";
  import NotesTab from "#components/video/study/NotesTab.svelte";
  import TranscriptTab from "#components/video/study/TranscriptTab.svelte";
  import LessonsTab from "./LessonsTab.svelte";
  import VIcon from "./VIcon.svelte";

  const TABS: { id: PanelTab; icon: string }[] = [
    { id: "lessons", icon: "courses" },
    { id: "notes", icon: "notes" },
    { id: "transcript", icon: "transcript" },
    { id: "ai", icon: "ask-ai" },
  ];

</script>

<aside class="side" aria-label={$t("video.panel.label") as string}>
  <div class="tabs" role="tablist">
    {#each TABS as tab (tab.id)}
      <button role="tab" aria-selected={player.panel === tab.id} class:on={player.panel === tab.id} onclick={() => setPanel(tab.id)}>
        <img src="/icons/video/{tab.icon}-32.webp" srcset="/icons/video/{tab.icon}-64.webp 2x" alt="" width="16" height="16" />
        <span>{$t(`video.panel.${tab.id}`)}</span>
        {#if tab.id === "notes" && player.notes.length}<small>{player.notes.length}</small>{/if}
      </button>
    {/each}
    <button class="x" onclick={() => togglePanel()} aria-label={$t("video.close") as string}><VIcon name="close" size={13} /></button>
  </div>
  <div class="body" role="tabpanel">
    {#if player.panel === "lessons"}<LessonsTab />
    {:else if player.panel === "notes"}<NotesTab />
    {:else if player.panel === "transcript"}<TranscriptTab />
    {:else if player.panel === "ai"}<AskTab />{/if}
  </div>
</aside>

<style>
  .side {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--pane-bg);
    border-left: var(--hairline) solid var(--content-border);
    color: var(--text);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    margin: 10px 10px 4px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .tabs button:not(.x) {
    all: unset;
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-width: 0;
    height: 28px;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    color: var(--text-dim);
    cursor: pointer;
  }
  .tabs button span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tabs button small {
    font-size: var(--text-caption);
    padding: 0 5px;
    border-radius: var(--radius-full);
    background: var(--fill-3);
  }
  .tabs button.on {
    background: var(--surface-hi);
    color: var(--text);
    font-weight: 600;
    box-shadow: var(--elev-1);
  }
  .tabs button:focus-visible {
    outline: var(--focus-ring);
  }
  .x {
    all: unset;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .x:hover {
    background: var(--fill-2);
  }
  .body {
    flex: 1;
    min-height: 0;
  }
  @media (max-width: 1100px) {
    .tabs button span {
      display: none;
    }
  }
</style>
