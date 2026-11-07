<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import SocialIcon from "./SocialIcon.svelte";
  import { ago, compact, type Platform, type Post } from "./api";
  import Self from "./PostCard.svelte";

  let {
    post,
    platform,
    nested = false,
    busy = false,
    onOpen,
    onAuthor,
    onLike,
    onRepost,
    onDownload,
    onExternal,
  }: {
    post: Post;
    platform: Platform;
    nested?: boolean;
    busy?: boolean;
    onOpen?: (p: Post) => void;
    onAuthor?: (handle: string) => void;
    onLike?: (p: Post) => void;
    onRepost?: (p: Post) => void;
    onDownload?: (p: Post) => void;
    onExternal?: (url: string) => void;
  } = $props();

  let playable = $derived(platform !== "tiktok");
  let grid = $derived(post.media.length > 1 ? "multi" : "single");

  function open(e: MouseEvent | KeyboardEvent) {
    const el = e.target as HTMLElement;
    if (el.closest("button, a, video")) return;
    onOpen?.(post);
  }
</script>

<div class="post" class:nested role="button" tabindex="0" onclick={open} onkeydown={(e) => e.key === "Enter" && open(e)}>
  {#if post.reposted_by}
    <div class="post-repost"><SocialIcon name="repost" size={13} /> {$t("social.post.reposted_by", { name: post.reposted_by.name || post.reposted_by.handle })}</div>
  {/if}
  <header class="post-head">
    <button class="post-author" onclick={() => onAuthor?.(post.author.handle)} title={`@${post.author.handle}`}>
      {#if post.author.avatar}
        <img class="avatar" src={post.author.avatar} alt="" width="36" height="36" referrerpolicy="no-referrer" loading="lazy" />
      {:else}
        <span class="avatar avatar-empty">{(post.author.name || post.author.handle || "?").slice(0, 1).toUpperCase()}</span>
      {/if}
      <span class="who">
        <span class="name">{post.author.name || post.author.handle}{#if post.author.verified}<span class="verified" title={$t("social.post.verified")}>✓</span>{/if}</span>
        <span class="handle">@{post.author.handle}{#if post.created_at} · {ago(post.created_at, $locale)}{/if}</span>
      </span>
    </button>
  </header>

  {#if post.text}<p class="post-text">{post.text}</p>{/if}

  {#if post.media.length && !nested}
    <div class="post-media {grid}">
      {#each post.media.slice(0, 4) as m, i (i)}
        {#if m.kind === "video" && playable}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={m.url} poster={m.thumb} controls preload="none" playsinline></video>
        {:else if m.kind === "video"}
          <button class="cover" onclick={() => onExternal?.(post.url)} aria-label={$t("social.post.open")}>
            {#if m.thumb}<img src={m.thumb} alt="" referrerpolicy="no-referrer" loading="lazy" />{/if}
            <span class="play"><SocialIcon name="play" size={22} /></span>
          </button>
        {:else}
          <img src={m.url} alt="" referrerpolicy="no-referrer" loading="lazy" />
        {/if}
      {/each}
      {#if post.media.length > 4}<span class="more">+{post.media.length - 4}</span>{/if}
    </div>
  {/if}

  {#if post.quoted && !nested}
    <Self post={post.quoted} {platform} nested onOpen={onOpen} onAuthor={onAuthor} />
  {/if}

  {#if !nested}
    <footer class="post-actions">
      <button class="act" class:on={post.liked} disabled={busy || !onLike} onclick={() => onLike?.(post)} aria-pressed={!!post.liked} title={$t(post.liked ? "social.post.unlike" : "social.post.like")}>
        <SocialIcon name="heart" /> <span>{compact(post.stats.likes, $locale)}</span>
      </button>
      <button class="act" onclick={() => onOpen?.(post)} title={$t("social.post.comments")}>
        <SocialIcon name="comment" /> <span>{compact(post.stats.replies, $locale)}</span>
      </button>
      {#if platform === "x"}
        <button class="act" class:on={post.reposted} disabled={busy || !onRepost} onclick={() => onRepost?.(post)} aria-pressed={!!post.reposted} title={$t(post.reposted ? "social.post.unrepost" : "social.post.repost")}>
          <SocialIcon name="repost" /> <span>{compact(post.stats.reposts, $locale)}</span>
        </button>
      {/if}
      {#if post.stats.views}
        <span class="act static" title={$t("social.post.views")}><SocialIcon name="views" /> <span>{compact(post.stats.views, $locale)}</span></span>
      {/if}
      <span class="grow"></span>
      {#if post.media.length && onDownload}
        <button class="act" onclick={() => onDownload?.(post)} title={$t("social.post.download")}><SocialIcon name="download" /></button>
      {/if}
      <button class="act" onclick={() => onExternal?.(post.url)} title={$t("social.post.open")}><SocialIcon name="external" /></button>
    </footer>
  {/if}
</div>

<style>
  .post {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
    text-align: left;
    min-width: 0;
  }

  .post:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .post.nested {
    padding: var(--space-2) var(--space-3);
    background: var(--fill-1);
    box-shadow: none;
  }

  .post-repost {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .post-author {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    cursor: pointer;
    min-width: 0;
    text-align: left;
  }

  .avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    flex-shrink: 0;
    object-fit: cover;
    background: var(--fill-2);
  }

  .nested .avatar {
    width: 22px;
    height: 22px;
  }

  .avatar-empty {
    display: grid;
    place-items: center;
    font-weight: 650;
    color: var(--text-dim);
  }

  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .nested .who {
    flex-direction: row;
    gap: 6px;
  }

  .name {
    font-weight: 650;
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .verified {
    margin-left: 4px;
    color: var(--accent);
    font-size: var(--text-xs);
  }

  .handle {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .post-text {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .nested .post-text {
    font-size: var(--text-sm);
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .post-media {
    position: relative;
    display: grid;
    gap: 4px;
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .post-media.multi {
    grid-template-columns: 1fr 1fr;
  }

  .post-media img,
  .post-media video,
  .cover {
    width: 100%;
    max-height: 460px;
    object-fit: cover;
    background: var(--fill-2);
    display: block;
  }

  .post-media.multi img,
  .post-media.multi video,
  .post-media.multi .cover {
    aspect-ratio: 1;
    max-height: 240px;
  }

  .cover {
    position: relative;
    padding: 0;
    border: 0;
    cursor: pointer;
    aspect-ratio: 9 / 12;
    max-width: 320px;
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .play {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: white;
    background: rgb(0 0 0 / 0.18);
  }

  .more {
    position: absolute;
    right: 8px;
    bottom: 8px;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgb(0 0 0 / 0.6);
    color: white;
    font-size: var(--text-xs);
  }

  .post-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: -6px;
  }

  .act {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }

  .act:hover:not(:disabled):not(.static) {
    background: var(--fill-1);
    color: var(--text);
  }

  .act.on {
    color: var(--danger, #e0245e);
  }

  .act:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .static {
    cursor: default;
  }

  .grow {
    flex: 1;
  }
</style>
