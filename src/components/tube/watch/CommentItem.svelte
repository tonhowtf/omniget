<script lang="ts">
  // One comment (spec §2.6): avatar 40, "@name" 13/500 + time, text with
  // timestamps, likes, creator heart, pinned label, and a thread of replies
  // that loads on demand (avatar 24, joined by a vertical line).
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar } from "$lib/tube/format";
  import type { Comment } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import { channelHref, errText, openChannel } from "../cards/util";
  import Runs from "./Runs.svelte";
  import Self from "./CommentItem.svelte";

  let {
    c,
    videoId,
    ownerAvatar = null,
    ownerName = "",
    onseek,
    reply = false,
  }: {
    c: Comment;
    videoId: string;
    ownerAvatar?: string | null;
    ownerName?: string;
    onseek: (t: number) => void;
    reply?: boolean;
  } = $props();

  let expanded = $state(false);
  let clamped = $state(false);
  let textEl = $state<HTMLDivElement>();
  let replies = $state<Comment[] | null>(null);
  let nextToken = $state<string | null>(null);
  let open = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    void c.text;
    if (textEl && !expanded) clamped = textEl.scrollHeight > textEl.clientHeight + 2;
  });

  async function loadReplies(token: string | null | undefined) {
    if (!token || busy) return;
    busy = true;
    error = null;
    try {
      const p = await api.comments(token);
      replies = [...(replies ?? []), ...p.items];
      nextToken = p.continuation ?? null;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function toggleReplies() {
    open = !open;
    if (open && replies === null) void loadReplies(c.replies_token);
  }

  let size = $derived(reply ? 24 : 40);
</script>

<div class="cm" class:reply>
  <a class="av" href={channelHref(c.author.id)} onclick={(e) => { e.preventDefault(); openChannel(c.author.id); }} tabindex="-1" aria-hidden="true">
    {#if c.author.avatar}
      <img class="yt-avatar" src={bestAvatar(c.author.avatar, size)} width={size} height={size} alt="" loading="lazy" referrerpolicy="no-referrer" />
    {:else}
      <span class="yt-avatar ph" style:width="{size}px" style:height="{size}px">{c.author.name.replace("@", "").slice(0, 1)}</span>
    {/if}
  </a>
  <div class="main">
    {#if c.pinned}
      <div class="pinned"><TIcon name="pin" size={14} />{$t("tube.comments.pinned_by", { name: ownerName })}</div>
    {/if}
    <div class="head">
      <a class="author" class:owner={c.is_owner} href={channelHref(c.author.id)} onclick={(e) => { e.preventDefault(); openChannel(c.author.id); }}>
        {c.author.name}
        {#if c.author.verified}<TIcon name="verified" size={12} />{/if}
      </a>
      {#if c.published_text}<span class="time">{c.published_text}</span>{/if}
    </div>
    <div class="text" class:clamp={!expanded} bind:this={textEl}>
      <Runs runs={c.text} {videoId} {onseek} />
    </div>
    {#if clamped || expanded}
      <button class="more" onclick={() => (expanded = !expanded)}>{$t(expanded ? "tube.comments.show_less" : "tube.comments.read_more")}</button>
    {/if}
    <div class="acts">
      <span class="like"><TIcon name="thumbs-up" size={16} />{c.likes_text ?? ""}</span>
      <span class="like"><TIcon name="thumbs-down" size={16} /></span>
      {#if c.hearted}
        <span class="heart" title={$t("tube.comments.hearted", { name: ownerName }) as string}>
          {#if ownerAvatar}<img class="yt-avatar" src={bestAvatar(ownerAvatar, 16)} width="16" height="16" alt="" referrerpolicy="no-referrer" />{/if}
          <span class="h"><TIcon name="heart-fill" size={12} stroke={0} /></span>
        </span>
      {/if}
    </div>
    {#if !reply && c.reply_count > 0 && c.replies_token}
      <button class="replies-btn" aria-expanded={open} onclick={toggleReplies}>
        <TIcon name={open ? "chevron-up" : "chevron-down"} size={20} />
        {$t("tube.comments.replies", { count: c.reply_count })}
      </button>
      {#if open}
        <div class="thread">
          {#if replies}
            {#each replies as r (r.id)}
              <Self c={r} {videoId} {ownerAvatar} {ownerName} {onseek} reply />
            {/each}
          {/if}
          {#if busy}
            <div class="yt-skel sk"></div>
          {:else if error}
            <button class="replies-btn" onclick={() => loadReplies(nextToken ?? c.replies_token)}>{$t("tube.common.retry")}</button>
          {:else if nextToken}
            <button class="replies-btn" onclick={() => loadReplies(nextToken)}>
              <TIcon name="chevron-down" size={20} />{$t("tube.comments.more_replies")}
            </button>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .cm {
    display: flex;
    gap: 16px;
  }
  .cm.reply {
    gap: 12px;
  }
  .av {
    flex: none;
  }
  .ph {
    display: grid;
    place-items: center;
    font: 500 14px/1 var(--yt-font);
    color: var(--yt-text-2);
    text-transform: uppercase;
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .pinned {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-bottom: 2px;
  }
  .author {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: 500 13px/18px var(--yt-font);
    color: var(--yt-text);
    text-decoration: none;
  }
  .author.owner {
    padding: 0 6px;
    border-radius: 999px;
    background: var(--yt-tonal-hover);
  }
  .author:hover {
    text-decoration: underline;
  }
  .time {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .text {
    font: var(--yt-type-meta);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .text.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .more {
    margin-top: 4px;
    padding: 0;
    border: 0;
    font: var(--yt-type-body-strong);
    color: var(--yt-text-2);
    background: none;
    cursor: pointer;
  }
  .more:hover {
    color: var(--yt-text);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 16px;
    height: 32px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .like {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .heart {
    position: relative;
    display: inline-flex;
  }
  .h {
    position: absolute;
    right: -6px;
    bottom: -4px;
    display: inline-flex;
    color: var(--yt-live-red);
  }
  .replies-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 36px;
    margin-left: -12px;
    padding: 0 12px;
    border: 0;
    border-radius: 999px;
    font: var(--yt-type-body-strong);
    color: var(--yt-link);
    background: transparent;
    cursor: pointer;
  }
  .replies-btn:hover {
    background: var(--yt-cta-tonal);
  }
  .replies-btn:focus-visible,
  .more:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .thread {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 8px;
    padding-left: 4px;
  }
  .thread::before {
    content: "";
    position: absolute;
    left: -44px;
    top: -12px;
    bottom: 20px;
    width: 2px;
    border-radius: 2px;
    background: var(--yt-divider);
  }
  .sk {
    height: 40px;
    width: 60%;
  }
</style>
