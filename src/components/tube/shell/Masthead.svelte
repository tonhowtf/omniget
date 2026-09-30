<script lang="ts">
  // 56px masthead (spec §2.2): ≡, brand, the "Videos | Music" segmented
  // switch (our addition), centered search, settings and the account.
  // Frosted when the content is scrolled; a thin accent bar runs along the
  // bottom while a view is loading.
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { back, canGoBack, go, goRoot, setFace, ui, type Face } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import AccountMenu from "./AccountMenu.svelte";
  import SearchBox from "./SearchBox.svelte";

  let { scrolled = false, loading = false, ontoggle }: { scrolled?: boolean; loading?: boolean; ontoggle: () => void } = $props();

  let face = $derived(ui.face);

  function home() {
    goRoot(face === "music" ? { name: "m_home" } : { name: "home" });
  }
</script>

<header class="mh" class:scrolled class:music={face === "music"}>
  <div class="start">
    <button class="ib" onclick={ontoggle} aria-label={$t("tube.shell.guide") as string} title={$t("tube.shell.guide") as string}>
      <TIcon name="menu" size={24} />
    </button>
    {#if canGoBack()}
      <button use:agentActions={["click"]} class="ib back" onclick={back} aria-label={$t("tube.shell.back") as string} title={$t("tube.shell.back") as string}>
        <TIcon name="back" size={22} />
      </button>
    {/if}
    <button use:agentActions={["click"]} class="brand" onclick={home} aria-label={$t("tube.shell.home") as string}>
      <img
        src={face === "music" ? "/icons/tube/tube-music-64.webp" : "/icons/tube/tube-64.webp"}
        srcset={face === "music" ? "/icons/tube/tube-music-64.webp 1x, /icons/tube/tube-music-128.webp 2x" : "/icons/tube/tube-64.webp 1x, /icons/tube/tube-128.webp 2x"}
        alt=""
        width="28"
        height="28"
      />
      <span class="word">OmniTube{#if face === "music"}<span class="sub"> Music</span>{/if}</span>
    </button>
    <div class="seg" role="tablist" aria-label={$t("tube.shell.face") as string}>
      <button use:agentActions={["click"]} role="tab" aria-selected={face === "video"} class:on={face === "video"} onclick={() => setFace("video")}>
        {$t("tube.shell.videos")}
      </button>
      <button use:agentActions={["click"]} role="tab" aria-selected={face === "music"} class:on={face === "music"} onclick={() => setFace("music")}>
        {$t("tube.shell.music")}
      </button>
    </div>
  </div>

  <div class="center">
    <SearchBox {face} />
  </div>

  <div class="end">
    <button use:agentActions={["click"]} class="ib" onclick={() => go({ name: "settings" })} aria-label={$t("tube.shell.settings") as string} title={$t("tube.shell.settings") as string}>
      <TIcon name="settings" size={22} />
    </button>
    <AccountMenu />
  </div>

  {#if loading}<span class="load" aria-hidden="true"></span>{/if}
</header>

<style>
  .mh {
    position: relative;
    display: flex;
    align-items: center;
    gap: 16px;
    height: var(--yt-masthead-h);
    padding: 0 16px;
    background: var(--yt-bg);
    z-index: 30;
    transition: background var(--yt-dur-base);
  }
  .mh.scrolled {
    background: var(--yt-frosted);
    backdrop-filter: blur(48px);
    -webkit-backdrop-filter: blur(48px);
  }
  .mh.music {
    height: var(--ytm-nav-h);
    background: transparent;
  }
  .mh.music.scrolled {
    background: var(--ytm-bg);
    backdrop-filter: none;
    box-shadow: 0 1px 0 var(--ytm-divider);
  }
  .start,
  .end {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .end {
    gap: 8px;
  }
  .center {
    flex: 1;
    display: flex;
    justify-content: center;
    min-width: 0;
    padding: 0 8px;
  }
  .ib {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--yt-text);
    cursor: pointer;
    flex: none;
    transition: background var(--yt-dur-fast);
  }
  .ib:hover {
    background: var(--yt-hover);
  }
  .ib:active {
    background: var(--yt-press);
  }
  .ib:focus-visible,
  .brand:focus-visible,
  .seg button:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 1px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 40px;
    margin-left: 8px;
    padding: 0 6px 0 2px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--yt-text-strong);
    cursor: pointer;
  }
  .brand img {
    width: 28px;
    height: 28px;
  }
  .word {
    font: 700 19px/1 var(--yt-font-display);
    letter-spacing: -0.02em;
    white-space: nowrap;
  }
  .sub {
    font-weight: 500;
    color: var(--yt-text-2);
  }
  .seg {
    position: relative;
    display: inline-flex;
    margin-left: 12px;
    padding: 3px;
    border-radius: 999px;
    background: var(--yt-tonal);
  }
  .seg button {
    position: relative;
    height: 30px;
    padding: 0 14px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--yt-text);
    font: var(--yt-type-body-strong);
    cursor: pointer;
    transition:
      color 200ms var(--yt-ease-standard),
      background-color 200ms var(--yt-ease-standard);
    white-space: nowrap;
  }
  .seg button:not(.on):hover {
    background: var(--yt-tonal);
  }
  .seg button.on {
    color: var(--yt-on-inverted);
    background: var(--yt-bg-inverted);
  }
  .load {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    width: 100%;
    background: var(--yt-accent);
    transform-origin: left;
    animation: load 1.6s var(--yt-ease-standard) infinite;
  }
  @keyframes load {
    0% {
      transform: scaleX(0);
      opacity: 1;
    }
    70% {
      transform: scaleX(0.85);
      opacity: 1;
    }
    100% {
      transform: scaleX(1);
      opacity: 0;
    }
  }

  @container omnitube (max-width: 980px) {
    .word .sub,
    .back {
      display: none;
    }
  }
  @container omnitube (max-width: 680px) {
    .word {
      display: none;
    }
    .seg {
      margin-left: 4px;
    }
  }
  @container omnitube (max-width: 600px) {
    .seg button {
      padding: 0 10px;
    }
    .end .ib {
      display: none;
    }
    .mh {
      gap: 8px;
      padding: 0 8px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .seg button {
      transition: none;
    }
    .load {
      animation-duration: 3s;
    }
  }
</style>
