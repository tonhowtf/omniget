<script lang="ts">
  /** Card compacto de um post do X, usado por thread, busca, perfil e favoritos. */
  import { t } from "$lib/i18n";
  import { openUrl } from "$lib/tools/rt";
  import { fmtDate, fmtN, type XPost } from "$lib/tools/x";

  let { post, index, total, compact = false }: { post: XPost; index?: number; total?: number; compact?: boolean } = $props();
</script>

<article class="post" class:compact>
  <header>
    {#if post.author.avatar}<img class="avatar" src={post.author.avatar} alt="" loading="lazy" />{/if}
    <div class="who">
      <span class="name">{post.author.name}</span>
      <span class="handle">@{post.author.handle}</span>
      {#if post.reposted_by}<span class="tag">↻ @{post.reposted_by}</span>{/if}
    </div>
    <span class="when">{#if index !== undefined && total}{index + 1}/{total} · {/if}{fmtDate(post.created_at)}</span>
  </header>
  {#if post.reply_to_handle}<div class="reply">{$t("tools.x.replying_to")} @{post.reply_to_handle}</div>{/if}
  <p class="text">{post.text}</p>
  {#if post.media.length}
    <div class="media" class:single={post.media.length === 1}>
      {#each post.media as m, i (m.url + i)}
        <button class="thumb" type="button" title={m.alt || m.kind} onclick={() => openUrl(m.url)}>
          {#if m.thumb || m.kind === "photo"}<img src={m.thumb || m.url} alt={m.alt} loading="lazy" />{/if}
          {#if m.kind !== "photo"}<span class="play">▶</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
  {#if post.quote}
    <blockquote>
      <b>@{post.quote.author.handle}</b> {post.quote.text}
    </blockquote>
  {/if}
  <footer>
    <span>♥ {fmtN(post.likes)}</span><span>↻ {fmtN(post.reposts)}</span><span>💬 {fmtN(post.replies)}</span>{#if post.views}<span>👁 {fmtN(post.views)}</span>{/if}
    <button class="link" type="button" onclick={() => openUrl(post.url)}>{$t("tools.x.open_on_x")}</button>
  </footer>
</article>

<style>
  .post { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3) var(--space-4); border-bottom: var(--hairline) solid var(--content-border); }
  .post:last-child { border-bottom: 0; }
  header { display: flex; align-items: center; gap: var(--space-2); }
  .avatar { width: 36px; height: 36px; border-radius: 50%; flex-shrink: 0; }
  .who { display: flex; align-items: baseline; gap: var(--space-1); min-width: 0; flex: 1; flex-wrap: wrap; }
  .name { font-weight: 600; color: var(--text); }
  .handle, .when, .reply { color: var(--text-muted); font-size: var(--text-sm); }
  .when { white-space: nowrap; }
  .text { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45; color: var(--text); }
  .compact .text { display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .media { display: grid; grid-template-columns: repeat(2, 1fr); gap: var(--space-1); }
  .media.single { grid-template-columns: 1fr; max-width: 420px; }
  .thumb { position: relative; padding: 0; border: 0; background: var(--surface-2, var(--surface)); border-radius: var(--radius-md); overflow: hidden; aspect-ratio: 16 / 10; cursor: pointer; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play { position: absolute; inset: 0; display: grid; place-items: center; color: #fff; font-size: 28px; text-shadow: 0 2px 8px rgba(0, 0, 0, 0.6); }
  blockquote { margin: 0; padding: var(--space-2) var(--space-3); border-left: 3px solid var(--content-border); color: var(--text-muted); font-size: var(--text-sm); white-space: pre-wrap; }
  footer { display: flex; gap: var(--space-3); font-size: var(--text-sm); color: var(--text-muted); align-items: center; }
  .link { margin-left: auto; background: none; border: 0; color: var(--accent-hi); cursor: pointer; font: inherit; padding: 0; }
  .link:hover { text-decoration: underline; }
  .tag { font-size: var(--text-xs); }
</style>
