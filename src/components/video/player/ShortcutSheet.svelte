<script lang="ts">
  import { t } from "$lib/i18n";
  import { KEY_GROUPS, keyLabel, keymap, rebind, resetKeys, type KeyAction } from "$lib/video/keys.svelte";
  import VIcon from "./VIcon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let listening = $state<KeyAction | null>(null);

  function capture(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!listening) {
      if (e.key === "Escape" || e.key === "?") onclose();
      return;
    }
    if (e.key === "Escape") {
      listening = null;
      return;
    }
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key) || /^[0-9]$/.test(e.key)) return;
    rebind(listening, e.key);
    listening = null;
  }
</script>

<svelte:window onkeydowncapture={capture} />

<div class="scrim" role="presentation" onclick={onclose}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label={$t("video.keys.title") as string} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <div class="head">
      <h2>{$t("video.keys.title")}</h2>
      <button class="btn btn-sm" onclick={resetKeys}>{$t("video.keys.reset")}</button>
      <button class="x" onclick={onclose} aria-label={$t("video.close") as string}><VIcon name="close" size={15} /></button>
    </div>
    <p class="hint">{$t("video.keys.hint")}</p>
    <div class="groups">
      {#each KEY_GROUPS as g (g.id)}
        <section>
          <h3>{$t(`video.keys.group_${g.id}`)}</h3>
          <ul>
            {#each g.actions as a (a)}
              <li>
                <span>{$t(`video.keys.${a}`)}</span>
                <button class="kbd-btn" class:listening={listening === a} onclick={() => (listening = listening === a ? null : a)}>
                  {listening === a ? $t("video.keys.press") : keymap[a] ? keyLabel(keymap[a]) : "—"}
                </button>
              </li>
            {/each}
            {#if g.id === "playback"}
              <li><span>{$t("video.keys.digits")}</span><span class="kbd-static">0–9</span></li>
              <li><span>{$t("video.keys.hold_shift")}</span><span class="kbd-static">⇧</span></li>
            {/if}
          </ul>
        </section>
      {/each}
    </div>
  </div>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--pl-shade) 45%, transparent);
    z-index: 40;
  }
  .sheet {
    width: min(860px, 92%);
    max-height: 84%;
    overflow-y: auto;
    padding: 20px 22px;
    border-radius: var(--radius-2xl);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-3);
    color: var(--text);
    outline: none;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
  }
  .x {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    cursor: pointer;
  }
  .x:hover {
    background: var(--fill-2);
  }
  .x:focus-visible,
  .kbd-btn:focus-visible {
    outline: var(--focus-ring);
  }
  .hint {
    margin: 4px 0 14px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .groups {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 16px 24px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--text-muted);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 28px;
    font-size: var(--text-sm);
  }
  .kbd-btn,
  .kbd-static {
    all: unset;
    min-width: 26px;
    padding: 2px 8px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    box-shadow: inset 0 -1px 0 var(--fill-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    text-align: center;
    white-space: nowrap;
  }
  .kbd-btn {
    cursor: pointer;
  }
  .kbd-btn:hover {
    background: var(--fill-3);
  }
  .kbd-btn.listening {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
</style>
