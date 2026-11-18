<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import DcAvatar from "./DcAvatar.svelte";
  import type { DcChannel, DcReadState } from "./api";
  import { dmIconUrl, isUnread } from "./format";

  let {
    title,
    channels,
    readStates,
    selected,
    isDms,
    onSelect,
  }: {
    title: string;
    channels: DcChannel[];
    readStates: Record<string, DcReadState>;
    selected: string | null;
    isDms: boolean;
    onSelect: (c: DcChannel) => void;
  } = $props();

  let query = $state("");
  let collapsed = $state<Record<string, boolean>>({});

  let rows = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (q) return channels.filter((c) => c.kind !== 4 && c.name.toLowerCase().includes(q));
    const out: DcChannel[] = [];
    let hide = false;
    for (const c of channels) {
      if (c.kind === 4) {
        hide = !!collapsed[c.id];
        out.push(c);
      } else if (!hide || c.id === selected) {
        out.push(c);
      }
    }
    return out;
  });

  function glyph(c: DcChannel): string | null {
    if (c.kind === 5) return "bolt";
    if (c.kind >= 10 && c.kind <= 12) return "forward";
    return null;
  }
</script>

<div class="channels">
  <div class="c-head">
    <h3 title={title}>{title}</h3>
  </div>
  <input class="input input-search" type="search" bind:value={query} placeholder={$t(isDms ? "guildchat.channels.search_dms" : "guildchat.channels.search")} />
  <div class="c-list" role="listbox" aria-label={title}>
    {#each rows as c (c.id)}
      {#if c.kind === 4}
        <button class="cat" aria-expanded={!collapsed[c.id]} onclick={() => (collapsed = { ...collapsed, [c.id]: !collapsed[c.id] })}>
          <span class="chev" class:open={!collapsed[c.id]}><TgIcon name="chevron" size={10} /></span>
          {c.name}
        </button>
      {:else}
        {@const rs = readStates[c.id]}
        {@const unread = isUnread(c, rs)}
        <button
          class="ch"
          class:sel={c.id === selected}
          class:unread
          class:thread={c.kind >= 10 && c.kind <= 12}
          role="option"
          aria-selected={c.id === selected}
          onclick={() => onSelect(c)}
          title={c.topic || c.name}
        >
          {#if isDms}
            <DcAvatar src={dmIconUrl(c, 64)} name={c.name} size={28} />
          {:else}
            {@const gl = glyph(c)}
            <span class="g">{#if gl}<TgIcon name={gl} size={13} />{:else}#{/if}</span>
          {/if}
          <span class="name">{c.name}</span>
          {#if rs?.mention_count}<span class="badge badge-danger">{rs.mention_count > 99 ? "99+" : rs.mention_count}</span>{/if}
        </button>
      {/if}
    {:else}
      <p class="none">{$t(query ? "guildchat.channels.no_match" : isDms ? "guildchat.channels.no_dms" : "guildchat.channels.none")}</p>
    {/each}
  </div>
</div>

<style>
  .channels {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    min-width: 0;
  }

  .c-head h3 {
    margin: 0;
    padding: 0 var(--space-1);
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .c-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .cat {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: var(--space-2);
    padding: 2px var(--space-1);
    border: none;
    background: none;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
    text-align: left;
    cursor: pointer;
  }

  .cat:hover {
    color: var(--text);
  }

  .chev {
    display: inline-flex;
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .ch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 30px;
    padding: 4px var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: none;
    font: inherit;
    font-size: var(--text-base);
    text-align: left;
    color: var(--text-dim);
    cursor: pointer;
  }

  .ch.thread {
    padding-left: var(--space-6);
    font-size: var(--text-sm);
  }

  .ch:hover {
    background: var(--fill-1);
    color: var(--text);
  }

  .ch.sel {
    background: var(--accent-soft);
    color: var(--text);
  }

  .ch.unread {
    color: var(--text);
    font-weight: 600;
  }

  .ch:focus-visible,
  .cat:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .g {
    display: inline-flex;
    justify-content: center;
    width: 14px;
    font-size: var(--text-md);
    line-height: 1;
    color: var(--text-faint);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .none {
    margin: var(--space-3) var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
