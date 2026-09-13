<script lang="ts">
  type Props = {
    open: boolean;
    onClose: () => void;
  };

  import { t } from "$lib/i18n";

  let { open, onClose }: Props = $props();

  type Shortcut = { keys: string[]; label: string };
  type Group = { title: string; items: Shortcut[] };

  const groups = $derived([
    {
      title: $t("study.player.keys.g_playback"),
      items: [
        { keys: [$t("study.player.keys.key_space")], label: $t("study.player.keys.play_pause") },
        { keys: ["F"], label: $t("study.player.keys.fullscreen") },
        { keys: ["M"], label: $t("study.player.keys.mute_unmute") },
        { keys: ["T"], label: $t("study.player.keys.cinema_mode") },
        { keys: ["Esc"], label: $t("study.player.keys.exit_fullscreen") },
      ],
    },
    {
      title: $t("study.player.keys.g_navigation"),
      items: [
        { keys: ["←", "J"], label: $t("study.player.keys.back_10") },
        { keys: ["→", "L", "K"], label: $t("study.player.keys.fwd_10") },
        { keys: ["Shift", "+", "J"], label: $t("study.player.keys.back_3") },
        { keys: ["Shift", "+", "L"], label: $t("study.player.keys.fwd_3") },
        { keys: [","], label: $t("study.player.keys.prev_frame") },
        { keys: ["."], label: $t("study.player.keys.next_frame") },
        { keys: ["0", "—", "9"], label: $t("study.player.keys.jump_pct") },
      ],
    },
    {
      title: $t("study.player.keys.g_speed"),
      items: [
        { keys: ["["], label: $t("study.player.keys.speed_down") },
        { keys: ["]"], label: $t("study.player.keys.speed_up") },
      ],
    },
    {
      title: $t("study.player.keys.g_subs_notes"),
      items: [
        { keys: ["C"], label: $t("study.player.keys.next_subtitle") },
        { keys: ["N"], label: $t("study.player.keys.add_note_ts") },
      ],
    },
    {
      title: $t("study.player.keys.g_general"),
      items: [
        { keys: ["?"], label: $t("study.player.keys.show_panel") },
        { keys: ["/"], label: $t("study.player.keys.search_anywhere") },
      ],
    },
  ]);

  function onBackdropKey(e: KeyboardEvent) {
    if (e.key === "Escape" || e.key === "?" || e.key === "/") {
      e.preventDefault();
      onClose();
    }
  }
</script>

{#if open}
  <div
    class="overlay"
    role="dialog"
    aria-modal="true"
    aria-label="Atalhos de teclado"
    tabindex="-1"
    onkeydown={onBackdropKey}
  >
    <button type="button" class="bg-btn" aria-label="Fechar" onclick={onClose}></button>
    <div class="modal" role="document">
      <header class="head">
        <h2>Atalhos de teclado</h2>
        <button type="button" class="close" aria-label="Fechar" onclick={onClose}>×</button>
      </header>
      <div class="body">
        {#each groups as g (g.title)}
          <section class="group">
            <h3>{g.title}</h3>
            <ul>
              {#each g.items as s, i (i)}
                <li>
                  <span class="keys">
                    {#each s.keys as k, j (j)}
                      {#if k === "+" || k === "—"}
                        <span class="sep">{k}</span>
                      {:else}
                        <kbd>{k}</kbd>
                      {/if}
                    {/each}
                  </span>
                  <span class="label">{s.label}</span>
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>
      <footer class="foot">
        Pressione <kbd>?</kbd> ou <kbd>Esc</kbd> para fechar
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 250;
    display: grid;
    place-items: center;
    background: color-mix(in oklab, black 60%, transparent);
    animation: fade-in 180ms ease-out;
  }

  .bg-btn {
    position: absolute;
    inset: 0;
    background: transparent;
    border: none;
    cursor: default;
  }

  .modal {
    position: relative;
    width: min(640px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    background: color-mix(in oklab, black 86%, transparent);
    backdrop-filter: blur(16px);
    border: 1px solid color-mix(in oklab, white 14%, transparent);
    border-radius: 14px;
    box-shadow: 0 24px 64px color-mix(in oklab, black 50%, transparent);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: white;
    z-index: 1;
    animation: pop-in 180ms ease-out;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid color-mix(in oklab, white 10%, transparent);
  }

  .head h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  .close {
    background: transparent;
    border: none;
    color: inherit;
    font-size: 24px;
    line-height: 1;
    padding: 4px 8px;
    border-radius: 6px;
    cursor: pointer;
  }

  .close:hover {
    background: color-mix(in oklab, white 12%, transparent);
  }

  .body {
    overflow-y: auto;
    padding: 16px 20px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px 32px;
  }

  @media (max-width: 600px) {
    .body {
      grid-template-columns: 1fr;
    }
  }

  .group h3 {
    margin: 0 0 8px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
    color: color-mix(in oklab, white 60%, transparent);
  }

  .group ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .group li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }

  .keys {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
  }

  .sep {
    color: color-mix(in oklab, white 50%, transparent);
    font-size: 11px;
  }

  kbd {
    display: inline-block;
    min-width: 22px;
    text-align: center;
    padding: 2px 6px;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    font-weight: 600;
    background: color-mix(in oklab, white 14%, transparent);
    border: 1px solid color-mix(in oklab, white 18%, transparent);
    border-bottom-width: 2px;
    border-radius: 4px;
    color: white;
  }

  .label {
    color: color-mix(in oklab, white 80%, transparent);
    text-align: right;
  }

  .foot {
    padding: 10px 20px;
    border-top: 1px solid color-mix(in oklab, white 10%, transparent);
    text-align: center;
    font-size: 12px;
    color: color-mix(in oklab, white 60%, transparent);
  }

  .foot kbd {
    margin: 0 2px;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes pop-in {
    from {
      opacity: 0;
      transform: scale(0.96);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .overlay,
    .modal {
      animation: none;
    }
  }
</style>
