<script lang="ts">
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import DcAvatar from "./DcAvatar.svelte";
  import type { DcAttachment, DcEmbed, DcMessage } from "./api";
  import { avatarUrl, displayName, fitBox, formatBytes, isImage, isVideo, mentionsMe, thumbUrl, tokenize, type Lookup } from "./format";

  let {
    m,
    grouped,
    look,
    canSend,
    onReply,
    onEdit,
    onDelete,
    onDownload,
  }: {
    m: DcMessage;
    grouped: boolean;
    look: Lookup;
    canSend: boolean;
    onReply: (m: DcMessage) => void;
    onEdit: (m: DcMessage) => void;
    onDelete: (m: DcMessage) => void;
    onDownload: (m: DcMessage) => void;
  } = $props();

  let confirming = $state(false);
  let own = $derived(m.author.id === look.meId);
  let mine = $derived(mentionsMe(m, look.meId));
  let system = $derived(m.type !== 0 && m.type !== 19);
  let tokens = $derived(m.content ? tokenize(m.content, look) : []);
  let refTokens = $derived(m.referenced_message?.content ? tokenize(m.referenced_message.content, look) : []);
  let hasMedia = $derived(m.attachments.length > 0 || m.embeds.some((e) => e.image || e.thumbnail || e.video));
  let time = $derived(new Date(m.timestamp));

  const fmtTime = (d: Date) => d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const fmtFull = (d: Date) => d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  const color = (e: DcEmbed) => (e.color != null ? `#${e.color.toString(16).padStart(6, "0")}` : "var(--separator)");

  function media(e: DcEmbed) {
    return e.image ?? (e.type === "image" || e.type === "gifv" ? e.thumbnail : null) ?? null;
  }

  function del(ev: MouseEvent) {
    if (ev.shiftKey || confirming) {
      confirming = false;
      onDelete(m);
    } else {
      confirming = true;
    }
  }
</script>

{#snippet text(list: ReturnType<typeof tokenize>)}
  {#each list as tk, i (i)}
    {#if tk.t === "text"}{tk.v}{:else if tk.t === "bold"}<strong>{tk.v}</strong>{:else if tk.t === "code"}<code>{tk.v}</code>{:else if tk.t === "block"}<pre>{tk.v}</pre>{:else if tk.t === "link"}<a class="ext" href={tk.v}>{tk.v}</a>{:else if tk.t === "mention"}<span class="mention" class:me={tk.me}>{tk.v}</span>{:else if tk.t === "channel" || tk.t === "role"}<span class="mention">{tk.v}</span>{:else if tk.t === "emoji"}<img class="emoji" src={tk.src} alt={tk.v} title={tk.v} loading="lazy" />{/if}
  {/each}
{/snippet}

{#snippet attachment(a: DcAttachment)}
  {#if isImage(a)}
    {@const box = fitBox(a.width, a.height)}
    <a class="ext img" href={a.url} title={a.filename} style:width="{box.width}px" style:height="{box.height}px">
      <img src={thumbUrl(a, box)} alt={a.filename} width={box.width} height={box.height} loading="lazy" decoding="async" />
    </a>
  {:else if isVideo(a)}
    {@const box = fitBox(a.width, a.height)}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video class="vid" src={a.url} controls preload="none" width={box.width} height={box.height}></video>
  {:else}
    <a class="ext file surface-card" href={a.url} title={a.filename}>
      <TgIcon name={a.content_type?.startsWith("audio/") ? "music" : "file"} size={18} />
      <span class="f-name">{a.filename}</span>
      <span class="f-size">{formatBytes(a.size)}</span>
    </a>
  {/if}
{/snippet}

<div class="row" class:grouped class:mine class:system data-id={m.id}>
  {#if m.referenced_message}
    {@const r = m.referenced_message}
    <div class="ref">
      <span class="ref-line" aria-hidden="true"></span>
      <DcAvatar src={avatarUrl(r.author, 32)} name={displayName(r.author)} size={16} />
      <span class="ref-name">{displayName(r.author)}</span>
      <span class="ref-text">{#if r.content}{@render text(refTokens)}{:else}<em>{$t("guildchat.messages.attachment_only")}</em>{/if}</span>
    </div>
  {/if}
  <div class="main">
    <div class="gutter">
      {#if grouped}
        <time class="side-time" datetime={m.timestamp} title={fmtFull(time)}>{fmtTime(time)}</time>
      {:else}
        <DcAvatar src={avatarUrl(m.author, 80)} name={displayName(m.author)} size={36} />
      {/if}
    </div>
    <div class="body">
      {#if !grouped}
        <div class="meta">
          <span class="author">{displayName(m.author)}</span>
          {#if m.author.bot}<span class="tag tag-info">{$t("guildchat.messages.bot")}</span>{/if}
          <time datetime={m.timestamp} title={fmtFull(time)}>{fmtFull(time)}</time>
        </div>
      {/if}
      {#if system}
        <div class="content sys">{m.content || $t("guildchat.messages.system")}</div>
      {:else if tokens.length}
        <div class="content">{@render text(tokens)}{#if m.edited_timestamp}<span class="edited" title={fmtFull(new Date(m.edited_timestamp))}> {$t("guildchat.messages.edited")}</span>{/if}</div>
      {/if}
      {#if m.attachments.length}
        <div class="atts">
          {#each m.attachments as a (a.id)}{@render attachment(a)}{/each}
        </div>
      {/if}
      {#each m.embeds as e, i (i)}
        {@const pic = media(e)}
        {#if e.title || e.description || e.author}
          <div class="embed" style:border-left-color={color(e)}>
            {#if e.author?.name}<div class="e-author">{e.author.name}</div>{/if}
            {#if e.title}
              {#if e.url}<a class="ext e-title" href={e.url}>{e.title}</a>{:else}<div class="e-title">{e.title}</div>{/if}
            {/if}
            {#if e.description}<div class="e-desc">{e.description}</div>{/if}
            {#if pic}
              {@const box = fitBox(pic.width, pic.height, 360, 240)}
              <img class="e-img" src={thumbUrl(pic, box)} alt="" width={box.width} height={box.height} loading="lazy" decoding="async" />
            {/if}
          </div>
        {:else if pic}
          {@const box = fitBox(pic.width, pic.height)}
          <a class="ext img" href={e.url || pic.url} style:width="{box.width}px" style:height="{box.height}px">
            <img src={thumbUrl(pic, box)} alt="" width={box.width} height={box.height} loading="lazy" decoding="async" />
          </a>
        {/if}
      {/each}
      {#if m.sticker_items.length}
        <div class="stickers">
          {#each m.sticker_items as s (s.id)}<span class="tag">{$t("guildchat.messages.sticker", { name: s.name })}</span>{/each}
        </div>
      {/if}
    </div>
  </div>
  {#if !system}
    <div class="actions" role="toolbar" aria-label={$t("guildchat.messages.actions")}>
      {#if confirming}
        <span class="ask">{$t("guildchat.messages.delete_confirm")}</span>
        <button class="btn btn-destructive btn-sm" onclick={del}>{$t("guildchat.messages.delete")}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (confirming = false)}>{$t("guildchat.common.cancel")}</button>
      {:else}
        {#if canSend}
          <button class="btn btn-ghost btn-sm btn-icon" title={$t("guildchat.messages.reply")} aria-label={$t("guildchat.messages.reply")} onclick={() => onReply(m)}><TgIcon name="forward" /></button>
        {/if}
        {#if hasMedia}
          <button class="btn btn-ghost btn-sm btn-icon" title={$t("guildchat.messages.download")} aria-label={$t("guildchat.messages.download")} onclick={() => onDownload(m)}><TgIcon name="download" /></button>
        {/if}
        {#if own && (m.type === 0 || m.type === 19)}
          <button class="btn btn-ghost btn-sm btn-icon" title={$t("guildchat.messages.edit")} aria-label={$t("guildchat.messages.edit")} onclick={() => onEdit(m)}><TgIcon name="edit" /></button>
          <button class="btn btn-ghost btn-sm btn-icon danger" title={$t("guildchat.messages.delete_hint")} aria-label={$t("guildchat.messages.delete")} onclick={del}><TgIcon name="trash" /></button>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .row {
    position: relative;
    padding: 2px var(--space-4) 2px 0;
    margin-top: var(--space-3);
    border-radius: var(--radius-sm);
    contain: layout style;
  }

  .row.grouped {
    margin-top: 0;
  }

  .row:hover {
    background: var(--fill-1);
  }

  .row.mine {
    background: color-mix(in srgb, var(--warning) 9%, transparent);
    box-shadow: inset 2px 0 0 var(--warning);
  }

  .main {
    display: flex;
    gap: var(--space-3);
  }

  .gutter {
    width: 56px;
    flex-shrink: 0;
    display: flex;
    justify-content: center;
    padding-top: 2px;
  }

  .side-time {
    visibility: hidden;
    font-size: var(--text-caption);
    color: var(--text-faint);
    line-height: 20px;
  }

  .row:hover .side-time {
    visibility: visible;
  }

  .body {
    flex: 1;
    min-width: 0;
  }

  .meta {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }

  .author {
    font-weight: 600;
    font-size: var(--text-base);
    color: var(--text);
  }

  .meta time {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }

  .content {
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .content.sys {
    color: var(--text-dim);
    font-style: italic;
  }

  .content code {
    padding: 0 4px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: 0.9em;
  }

  .content pre,
  .ref-text pre {
    margin: 4px 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    white-space: pre-wrap;
  }

  .mention {
    padding: 0 2px;
    border-radius: var(--radius-xs);
    background: color-mix(in srgb, var(--blue) 18%, transparent);
    color: var(--blue);
    font-weight: 500;
  }

  .mention.me {
    background: color-mix(in srgb, var(--warning) 22%, transparent);
    color: var(--text);
  }

  .emoji {
    width: 1.375em;
    height: 1.375em;
    vertical-align: -0.3em;
    object-fit: contain;
  }

  .edited {
    font-size: var(--text-caption);
    color: var(--text-faint);
  }

  a.ext {
    color: var(--accent-hi);
    text-decoration: none;
  }

  a.ext:hover {
    text-decoration: underline;
  }

  .atts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  .img {
    display: block;
    overflow: hidden;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    max-width: 100%;
  }

  .img img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .vid {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-md);
    background: #000;
  }

  .file {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 380px;
    padding: var(--space-2) var(--space-3);
    color: var(--text);
  }

  .f-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--accent-hi);
  }

  .f-size {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .embed {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 480px;
    margin-top: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border-left: 3px solid;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .e-author {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text);
  }

  .e-title {
    font-weight: 600;
  }

  .e-desc {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 8;
    line-clamp: 8;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .e-img {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-sm);
  }

  .stickers {
    display: flex;
    gap: var(--space-1);
    margin-top: var(--space-1);
  }

  .ref {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 56px;
    margin-left: var(--space-3);
    font-size: var(--text-xs);
    color: var(--text-dim);
    min-width: 0;
    position: relative;
  }

  .ref-line {
    position: absolute;
    left: 28px;
    top: 50%;
    width: 24px;
    height: 10px;
    border-left: 2px solid var(--separator);
    border-top: 2px solid var(--separator);
    border-top-left-radius: 6px;
  }

  .ref-name {
    font-weight: 600;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .ref-text {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ref-text pre {
    display: inline;
  }

  .actions {
    position: absolute;
    top: -14px;
    right: var(--space-3);
    display: none;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--popup-bg);
    box-shadow: 0 0 0 var(--hairline) var(--content-border), var(--elev-1);
    z-index: 2;
  }

  .row:hover .actions,
  .actions:focus-within {
    display: flex;
  }

  .ask {
    padding: 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .danger:hover {
    color: var(--error);
  }
</style>
