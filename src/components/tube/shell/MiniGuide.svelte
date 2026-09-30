<script lang="ts">
  // 72px mini guide (spec §2.3): icon 24 + 10px label per entry, filled icon
  // on the active one.
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { current, goRoot, ui } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import { MUSIC_MAIN, VIDEO_MAIN, activeId } from "./nav";

  let music = $derived(ui.face === "music");
  let active = $derived.by(() => {
    void ui.nav;
    return activeId(current());
  });
  let entries = $derived(music ? MUSIC_MAIN : VIDEO_MAIN.filter((e) => !(e.id === "shorts" && tubePrefs().hide_shorts)));
</script>

<nav class="mini" aria-label={$t("tube.shell.guide") as string}>
  {#each entries as e (e.id)}
    {@const on = active === e.id}
    <button use:agentActions={["click"]} class="it" class:on aria-current={on ? "page" : undefined} onclick={() => e.route && goRoot(e.route)}>
      <TIcon name={on && e.iconOn ? e.iconOn : e.icon} size={24} />
      <span>{$t(e.label)}</span>
    </button>
  {/each}
</nav>

<style>
  .mini {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: var(--yt-mini-guide-w);
    padding: 4px 4px 0;
  }
  .it {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 64px;
    padding: 16px 0 14px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--yt-text);
    cursor: pointer;
    transition: background var(--yt-dur-fast);
  }
  .it span {
    max-width: 60px;
    font: var(--yt-type-mini-guide);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .it:hover {
    background: var(--yt-hover);
  }
  .it.on {
    background: var(--yt-tonal);
  }
  .it:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: -2px;
  }
</style>
