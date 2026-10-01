<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/video/api";
  import { app, setIncognito } from "$lib/video/app.svelte";
  import type { SourceInfo } from "$lib/video/types";
  import { goRoot, ui, type View } from "$lib/video/ui.svelte";
  import VIcon from "./player/VIcon.svelte";

  let { onsettings }: { onsettings: () => void } = $props();

  type Item = { name: View["name"]; icon: string };
  const LIBRARY: Item[] = [
    { name: "home", icon: "video" },
    { name: "movies", icon: "movies" },
    { name: "shows", icon: "series" },
    { name: "courses", icon: "courses" },
    { name: "folders", icon: "folders" },
    { name: "history", icon: "history" },
  ];
  const ONLINE: Item[] = [
    { name: "live", icon: "live" },
    { name: "anime", icon: "anime" },
    { name: "addons", icon: "addons" },
    { name: "sources", icon: "subscriptions" },
  ];

  let servers = $state<SourceInfo[]>([]);

  $effect(() => {
    void app.version;
    api
      .sources()
      .then((s) => (servers = s.filter((x) => x.kind === "jellyfin")))
      .catch(() => (servers = []));
  });

  function active(name: View["name"]): boolean {
    const v = ui.view.name;
    if (v === name) return true;
    if (name === "shows" && v === "show") return true;
    if (name === "courses" && v === "course") return true;
    return false;
  }
</script>

<nav class="side" aria-label={$t("video.nav.label") as string}>
  <div class="brand">
    <img src="/icons/menu/video-64.webp" srcset="/icons/menu/video-64.webp 1x, /icons/menu/video-128.webp 2x" alt="" width="30" height="30" />
    <span>{$t("video.title")}</span>
  </div>

  <ul>
    {#each LIBRARY as n (n.name)}
      <li>
        <button class="ni" class:on={active(n.name)} aria-current={active(n.name) ? "page" : undefined} onclick={() => goRoot({ name: n.name } as View)} title={$t(`video.nav.${n.name}`) as string}>
          <img src="/icons/video/{n.icon}-32.webp" srcset="/icons/video/{n.icon}-64.webp 2x" alt="" width="18" height="18" />
          <span>{$t(`video.nav.${n.name}`)}</span>
        </button>
      </li>
    {/each}
  </ul>

  <h3 class="sec">{$t("video.nav.online")}</h3>
  <ul>
    {#each ONLINE as n (n.name)}
      <li>
        <button class="ni" class:on={active(n.name)} aria-current={active(n.name) ? "page" : undefined} onclick={() => goRoot({ name: n.name } as View)} title={$t(`video.nav.${n.name}`) as string}>
          <img src="/icons/video/{n.icon}-32.webp" srcset="/icons/video/{n.icon}-64.webp 2x" alt="" width="18" height="18" />
          <span>{$t(`video.nav.${n.name}`)}</span>
        </button>
      </li>
    {/each}
    {#each servers as s (s.id)}
      {@const on = ui.view.name === "jellyfin" && ui.view.source === s.id}
      <li>
        <button class="ni sub" class:on class:off={!s.enabled || s.status === "down"} onclick={() => goRoot({ name: "jellyfin", source: s.id, title: s.name })} title={s.name}>
          <img src="/icons/video/server-32.webp" srcset="/icons/video/server-64.webp 2x" alt="" width="16" height="16" />
          <span>{s.name}</span>
          <i class="st st-{s.enabled ? s.status : 'disabled'}" aria-label={$t(`video.nav.status_${s.enabled ? s.status : "disabled"}`) as string}></i>
        </button>
      </li>
    {/each}
  </ul>

  <h3 class="sec">{$t("video.nav.learn")}</h3>
  <ul class="grow">
    <li>
      <button class="ni" class:on={active("study")} aria-current={active("study") ? "page" : undefined} onclick={() => goRoot({ name: "study" })} title={$t("video.nav.study") as string}>
        <img src="/icons/video/flashcards-32.webp" srcset="/icons/video/flashcards-64.webp 2x" alt="" width="18" height="18" />
        <span>{$t("video.nav.study")}</span>
      </button>
    </li>
  </ul>

  {#if app.scan}
    <div class="scan" role="status">
      <span class="sl"><VIcon name="loader" size={12} spin /> {$t("video.scan.progress", { done: app.scan.done, total: app.scan.total })}</span>
      {#if app.scan.total > 0}<span class="sb"><span style:width="{(app.scan.done / app.scan.total) * 100}%"></span></span>{/if}
      {#if app.scan.current}<span class="sc" title={app.scan.current}>{app.scan.current}</span>{/if}
    </div>
  {/if}

  <div class="foot">
    <button class="ni" class:on={app.incognito} aria-pressed={app.incognito} onclick={() => setIncognito(!app.incognito)} title={$t("video.nav.incognito_hint") as string}>
      <VIcon name={app.incognito ? "eye-off" : "eye"} size={17} />
      <span>{app.incognito ? $t("video.nav.incognito_on") : $t("video.nav.incognito")}</span>
    </button>
    <button class="ni" onclick={onsettings} title={$t("video.nav.settings") as string}>
      <VIcon name="settings" size={17} />
      <span>{$t("video.nav.settings")}</span>
    </button>
  </div>
</nav>

<style>
  .side {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    height: 100%;
    overflow-y: auto;
    padding: var(--space-3) var(--space-2);
    border-right: var(--hairline) solid var(--content-border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px var(--space-3);
    font-family: var(--font-display);
    font-weight: 750;
    font-size: var(--text-lg);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .grow {
    flex: 1;
  }
  .sec {
    margin: var(--space-3) 0 2px;
    padding: 0 10px;
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .ni {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    color: var(--text-dim);
    cursor: pointer;
  }
  .ni img {
    flex-shrink: 0;
  }
  .ni span {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ni:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .ni.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-weight: 600;
  }
  .ni.sub {
    height: 28px;
    padding-left: 18px;
    font-size: var(--text-sm);
  }
  .ni.off {
    opacity: 0.6;
  }
  .ni:focus-visible {
    outline: var(--focus-ring);
  }
  .st {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--success);
  }
  .st-slow {
    background: var(--warning);
  }
  .st-down {
    background: var(--danger);
  }
  .st-disabled {
    background: var(--fill-4);
  }
  .scan {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: var(--space-2) 2px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .sl {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sb {
    height: 3px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
  }
  .sb span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--duration-base);
  }
  .sc {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-muted);
  }
  .foot {
    border-top: var(--hairline) solid var(--content-border);
    padding-top: var(--space-2);
    margin-top: var(--space-2);
  }
  @media (max-width: 980px) {
    .ni span,
    .brand span,
    .sec,
    .scan .sc,
    .st {
      display: none;
    }
    .ni,
    .ni.sub {
      justify-content: center;
      padding: 0;
    }
    .scan {
      padding: 6px;
    }
  }
</style>
