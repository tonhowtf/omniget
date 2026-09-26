<script lang="ts">
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import type { MoreAction } from "$lib/home/omnibox-controller";

  let { onPick }: { onPick: (action: MoreAction) => void } = $props();

  // Ported from the coss Menu (21st #11561): one trigger, a roving-focus list,
  // Esc and outside click close it and focus goes back to the trigger.
  const items: { action: MoreAction; key: string; icon: string }[] = [
    { action: "batch", key: "home.action_batch", icon: "M4 6h16M4 12h16M4 18h10" },
    { action: "torrent", key: "home.action_torrent", icon: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5" },
    { action: "p2p", key: "home.action_p2p", icon: "M22 2 11 13M22 2l-7 20-4-9-9-4z" },
    { action: "advanced", key: "home.action_advanced", icon: "m4 17 6-6-6-6M12 19h8" },
  ];

  let open = $state(false);
  let trigger = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLDivElement | null>(null);
  const menuId = "home-more-menu";

  async function show(focusIndex = 0) {
    open = true;
    await tick();
    menuItems()[focusIndex]?.focus();
  }

  function hide(returnFocus = true) {
    open = false;
    if (returnFocus) trigger?.focus();
  }

  function menuItems(): HTMLButtonElement[] {
    return menu ? [...menu.querySelectorAll<HTMLButtonElement>("[role=menuitem]")] : [];
  }

  function onTriggerKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void show(e.key === "ArrowUp" ? items.length - 1 : 0);
    }
  }

  function onMenuKey(e: KeyboardEvent) {
    const list = menuItems();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") { e.preventDefault(); list[(i + 1) % list.length]?.focus(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); list[(i - 1 + list.length) % list.length]?.focus(); }
    else if (e.key === "Home") { e.preventDefault(); list[0]?.focus(); }
    else if (e.key === "End") { e.preventDefault(); list[list.length - 1]?.focus(); }
    else if (e.key === "Escape") { e.preventDefault(); hide(); }
    else if (e.key === "Tab") { hide(false); }
  }

  function pick(action: MoreAction) {
    hide(false);
    onPick(action);
  }

  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (!menu?.contains(target) && !trigger?.contains(target)) hide(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });
</script>

<div class="more">
  <button
    bind:this={trigger}
    type="button"
    class="more-trigger"
    class:open
    aria-label={$t("home.more")}
    title={$t("home.more")}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? menuId : undefined}
    data-home-more
    onclick={() => (open ? hide() : show())}
    onkeydown={onTriggerKey}
  >
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
  </button>

  {#if open}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div bind:this={menu} id={menuId} class="more-menu" role="menu" aria-label={$t("home.more")} tabindex="-1" onkeydown={onMenuKey}>
      {#each items as item (item.action)}
        <button type="button" role="menuitem" class="more-item" tabindex="-1" onclick={() => pick(item.action)}>
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={item.icon} /></svg>
          <span>{$t(item.key)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .more {
    position: relative;
    flex-shrink: 0;
  }

  .more-trigger {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border: none;
    border-radius: 50%;
    background: var(--fill-1);
    color: var(--text-muted);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out), transform var(--duration-base) var(--ease-out);
  }

  @media (hover: hover) {
    .more-trigger:hover {
      background: var(--fill-2);
      color: var(--text);
    }
  }

  .more-trigger.open {
    background: var(--fill-2);
    color: var(--text);
  }

  .more-trigger.open svg {
    transform: rotate(45deg);
  }

  .more-trigger svg {
    transition: transform var(--duration-base) var(--ease-out);
  }

  .more-trigger:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  /* Grows out of the + (Morphing Popover feel, 21st #1670), CSS only. */
  .more-menu {
    position: absolute;
    top: calc(100% + 10px);
    left: -8px;
    z-index: 30;
    min-width: 232px;
    display: flex;
    flex-direction: column;
    padding: 5px;
    background: var(--popup-bg, var(--surface));
    border-radius: 14px;
    box-shadow: var(--elev-2);
    transform-origin: 26px 0;
    animation: menu-in 160ms var(--ease-out);
  }

  .more-item {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 0 10px;
    border: none;
    border-radius: 9px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .more-item svg {
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .more-item:hover,
  .more-item:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }

  .more-item:hover svg,
  .more-item:focus-visible svg {
    color: var(--accent-text, var(--accent));
  }

  @keyframes menu-in {
    from { opacity: 0; transform: scale(0.94) translateY(-4px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .more-menu { animation: none; }
    .more-trigger, .more-trigger svg { transition: none; }
  }
</style>
