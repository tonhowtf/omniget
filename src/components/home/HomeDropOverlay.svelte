<script lang="ts">
  import HomeHero from "$components/home/HomeHero.svelte";
  import { t } from "$lib/i18n";

  let { visible = false }: { visible?: boolean } = $props();
</script>

<!-- Full-pane drop target (21st extend-hq File Upload #15587, as an overlay).
     Pointer events stay off: the page listens for the drop itself. -->
{#if visible}
  <div class="drop-overlay" role="status" aria-live="polite">
    <div class="drop-frame">
      <HomeHero art="drop" size={144} />
      <p class="drop-title">{$t("home.drop_title")}</p>
      <p class="drop-hint">{$t("home.drop_hint")}</p>
    </div>
  </div>
{/if}

<style>
  .drop-overlay {
    position: absolute;
    inset: 10px;
    z-index: 40;
    display: grid;
    place-items: center;
    pointer-events: none;
    border-radius: 22px;
    background: var(--bg);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 60%, transparent);
    animation: overlay-in 140ms var(--ease-out);
  }

  .drop-frame {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
  }

  .drop-title {
    margin: 10px 0 0;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text);
  }

  .drop-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  @keyframes overlay-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (prefers-reduced-motion: reduce) {
    .drop-overlay { animation: none; }
  }
</style>
