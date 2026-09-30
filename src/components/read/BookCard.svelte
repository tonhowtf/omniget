<script lang="ts">
  import { t } from "$lib/i18n";
  import { coverUrl } from "$lib/read/library-store.svelte";
  import type { Book } from "$lib/read/types";
  import Cover from "./Cover.svelte";
  import ReadIcon from "./ReadIcon.svelte";
  import { agentActions } from "$lib/native-agent/actions";

  let {
    book,
    selected = false,
    onopen,
    onread,
    onmenu,
    onselect,
  }: {
    book: Book;
    selected?: boolean;
    onopen: () => void;
    onread?: () => void;
    onmenu?: (e: MouseEvent) => void;
    onselect?: (e: MouseEvent) => void;
  } = $props();

  let pct = $derived(Math.round(book.progress * 100));
</script>

<div
  data-agent-actions="click"
  use:agentActions={["click"]}
  class="bc"
  class:sel={selected}
  role="button"
  tabindex="0"
  onclick={(e) => (e.metaKey || e.ctrlKey || e.shiftKey ? onselect?.(e) : onopen())}
  ondblclick={() => onread?.()}
  onkeydown={(e) => {
    if (e.key === "Enter") onread ? onread() : onopen();
  }}
  oncontextmenu={(e) => onmenu?.(e)}
  title={book.title}
>
  <div class="art">
    <Cover src={coverUrl(book.cover)} title={book.title} />
    {#if book.status === "finished"}<span class="flag done"><ReadIcon name="check" size={12} /></span>
    {:else if pct > 0}<span class="flag">{pct}%</span>{/if}
    {#if book.favorite}<span class="fav"><ReadIcon name="heart-fill" size={13} /></span>{/if}
    {#if onread}
      <button
        data-agent-actions="click"
        use:agentActions={["click"]}
        class="play"
        onclick={(e) => {
          e.stopPropagation();
          onread?.();
        }}
        aria-label={$t("read.book.read") as string}><ReadIcon name="book-open" size={16} /></button
      >
    {/if}
    {#if pct > 0 && book.status !== "finished"}<div class="prog"><i style:width="{pct}%"></i></div>{/if}
  </div>
  <div class="t">{book.title}</div>
  <div class="a">
    {#if book.series && book.series_index != null}#{book.series_index} · {/if}{book.authors.join(", ") || book.format.toUpperCase()}
  </div>
</div>

<style>
  .bc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    cursor: pointer;
    outline: none;
    border-radius: 8px;
  }
  .bc:focus-visible .art,
  .bc.sel .art {
    outline: 2.5px solid var(--accent);
    outline-offset: 2px;
    border-radius: 6px;
  }
  .art {
    position: relative;
    transition: transform 0.15s;
  }
  .bc:hover .art {
    transform: translateY(-2px);
  }
  .flag {
    position: absolute;
    top: 6px;
    right: 6px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: rgb(0 0 0 / 0.62);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
  }
  .flag.done {
    display: grid;
    place-items: center;
    padding: 3px;
    background: var(--success, #2f9e52);
  }
  .fav {
    position: absolute;
    top: 6px;
    left: 6px;
    color: #ff5a7a;
    filter: drop-shadow(0 1px 1px rgb(0 0 0 / 0.5));
  }
  .play {
    all: unset;
    position: absolute;
    right: 8px;
    bottom: 12px;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    opacity: 0;
    transform: translateY(4px);
    transition:
      opacity 0.15s,
      transform 0.15s;
    box-shadow: 0 2px 8px rgb(0 0 0 / 0.35);
    cursor: pointer;
  }
  .bc:hover .play,
  .play:focus-visible {
    opacity: 1;
    transform: none;
  }
  .prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 4px;
    background: rgb(0 0 0 / 0.4);
  }
  .prog i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .t {
    font-size: var(--text-sm);
    font-weight: 600;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .a {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
