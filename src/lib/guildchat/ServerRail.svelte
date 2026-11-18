<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import DcAvatar from "./DcAvatar.svelte";
  import type { DcChannel, DcGuild, DcReadState } from "./api";
  import { guildIconUrl, isUnread } from "./format";

  let {
    guilds,
    dms,
    readStates,
    selected,
    onSelect,
  }: {
    guilds: DcGuild[];
    dms: DcChannel[];
    readStates: Record<string, DcReadState>;
    selected: string;
    onSelect: (id: string) => void;
  } = $props();

  function badge(list: DcChannel[]): { unread: boolean; mentions: number } {
    let unread = false;
    let mentions = 0;
    for (const c of list) {
      const rs = readStates[c.id];
      if (rs?.mention_count) mentions += rs.mention_count;
      if (!unread && isUnread(c, rs)) unread = true;
    }
    return { unread, mentions };
  }

  let dmBadge = $derived(badge(dms));
  let guildBadges = $derived(new Map(guilds.map((g) => [g.id, badge(g.channels)])));
</script>

<nav class="rail" aria-label={$t("guildchat.rail.label")}>
  <button class="item" class:sel={selected === "dms"} title={$t("guildchat.rail.dms")} aria-label={$t("guildchat.rail.dms")} aria-current={selected === "dms" ? "page" : undefined} onclick={() => onSelect("dms")}>
    <span class="dm-icon"><TgIcon name="inbox" size={18} /></span>
    {#if dmBadge.mentions}<span class="badge badge-danger count">{dmBadge.mentions > 99 ? "99+" : dmBadge.mentions}</span>{/if}
  </button>
  <span class="sep" aria-hidden="true"></span>
  {#each guilds as g (g.id)}
    {@const b = guildBadges.get(g.id)}
    <button class="item" class:sel={selected === g.id} title={g.name} aria-label={g.name} aria-current={selected === g.id ? "page" : undefined} onclick={() => onSelect(g.id)}>
      {#if b?.unread && selected !== g.id}<span class="pip" aria-hidden="true"></span>{/if}
      <DcAvatar src={guildIconUrl(g, 96)} name={g.name} size={40} square />
      {#if b?.mentions}<span class="badge badge-danger count">{b.mentions > 99 ? "99+" : b.mentions}</span>{/if}
    </button>
  {/each}
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    overflow-y: auto;
    scrollbar-width: none;
    min-height: 0;
  }

  .item {
    position: relative;
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    flex-shrink: 0;
    padding: 0;
    border: none;
    border-radius: var(--radius-lg);
    background: none;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .item:hover {
    background: var(--fill-1);
  }

  .item.sel {
    background: var(--accent-soft);
  }

  .item:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .dm-icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 30%;
    background: var(--fill-2);
    color: var(--text);
  }

  .sep {
    width: 24px;
    height: var(--hairline);
    background: var(--separator);
    flex-shrink: 0;
  }

  .pip {
    position: absolute;
    left: -2px;
    top: 50%;
    width: 4px;
    height: 8px;
    margin-top: -4px;
    border-radius: 0 4px 4px 0;
    background: var(--text);
  }

  .count {
    position: absolute;
    right: 0;
    bottom: 0;
    box-shadow: 0 0 0 2px var(--primary);
  }
</style>
