<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { WaChat } from "./api";
  import { wa } from "./session.svelte";
  import WaIcon from "./WaIcon.svelte";

  let { selected = $bindable<WaChat | null>(null) }: { selected: WaChat | null } = $props();

  let query = $state("");
  let list = $state<WaChat[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let seq = 0;

  async function load() {
    const mine = ++seq;
    try {
      const rows = await api.chats(query.trim());
      if (mine !== seq) return;
      list = rows;
      error = null;
      if (selected) {
        const fresh = rows.find((c) => c.jid === selected?.jid);
        if (fresh) selected = fresh;
      }
    } catch (e) {
      if (mine === seq) error = api.errText(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  $effect(() => {
    void wa.chatsVersion;
    void query;
    const timer = setTimeout(() => void load(), query ? 180 : 0);
    return () => clearTimeout(timer);
  });

  function when(ts: number | null): string {
    if (!ts) return "";
    const d = new Date(ts * 1000);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const diff = (now.getTime() - d.getTime()) / 86_400_000;
    if (diff < 7) return d.toLocaleDateString([], { weekday: "short" });
    return d.toLocaleDateString();
  }

  function preview(c: WaChat): string {
    const prefix = c.last_from_me ? `${$t("whatsapp.you")}: ` : "";
    if (c.last_text) return prefix + c.last_text;
    if (c.last_kind && c.last_kind !== "text") return prefix + $t(`whatsapp.kind.${c.last_kind}`);
    return "";
  }

  function kindIcon(kind: string | null): string | null {
    switch (kind) {
      case "image":
        return "photo";
      case "video":
        return "video";
      case "audio":
        return "music";
      case "voice":
        return "mic";
      case "document":
        return "file";
      case "sticker":
        return "sticker";
      case "location":
        return "pin";
      case "contact":
        return "contact";
      case "poll":
        return "poll";
      default:
        return null;
    }
  }

  function initials(name: string): string {
    const parts = name.replace(/[^\p{L}\p{N} ]/gu, "").trim().split(/\s+/);
    const s = (parts[0]?.[0] ?? "") + (parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : "");
    return s.toUpperCase() || "#";
  }
</script>

<aside class="chats">
  <div class="chats-search">
    <input class="input input-search" type="search" bind:value={query} placeholder={$t("whatsapp.chats.search")} aria-label={$t("whatsapp.chats.search")} />
  </div>

  {#if loading}
    <div class="chats-state"><span class="spinner"></span></div>
  {:else if error}
    <div class="chats-state">
      <div class="notice notice-warning">
        <div class="notice-body"><div class="notice-text">{error}</div></div>
      </div>
      <button class="btn btn-ghost btn-sm" onclick={() => void load()}>{$t("whatsapp.retry")}</button>
    </div>
  {:else if list.length === 0}
    <div class="chats-state">
      {#if query.trim()}
        <div class="empty-state-hint">{$t("whatsapp.chats.no_match")}</div>
      {:else}
        <span class="spinner"></span>
        <div class="empty-state-title">{$t("whatsapp.chats.empty_title")}</div>
        <div class="empty-state-hint">{$t("whatsapp.chats.empty_hint")}</div>
      {/if}
    </div>
  {:else}
    <ul class="chat-list" role="listbox" aria-label={$t("whatsapp.chats.title")}>
      {#each list as chat (chat.jid)}
        {@const icon = kindIcon(chat.last_kind)}
        <li>
          <button
            type="button"
            class="chat"
            class:active={selected?.jid === chat.jid}
            role="option"
            aria-selected={selected?.jid === chat.jid}
            onclick={() => (selected = chat)}
          >
            <span class="avatar" class:group={chat.is_group} aria-hidden="true">
              {#if chat.is_group}<WaIcon name="users" size={16} />{:else}{initials(chat.name)}{/if}
            </span>
            <span class="chat-main">
              <span class="chat-top">
                <span class="chat-name">{chat.name}</span>
                <span class="chat-time" class:unread={chat.unread > 0}>{when(chat.last_ts)}</span>
              </span>
              <span class="chat-bottom">
                <span class="chat-preview">
                  {#if icon}<WaIcon name={icon} size={12} />{/if}
                  {preview(chat)}
                </span>
                {#if chat.unread > 0}
                  <span class="unread-badge" aria-label={$t("whatsapp.chats.unread", { count: chat.unread })}>{chat.unread > 99 ? "99+" : chat.unread}</span>
                {/if}
              </span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</aside>

<style>
  .chats {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: var(--hairline) solid var(--separator);
  }

  .chats-search {
    padding: var(--space-3);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .chats-search .input {
    width: 100%;
  }

  .chats-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }

  .chat-list {
    list-style: none;
    margin: 0;
    padding: var(--space-1);
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .chat {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2) var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .chat:hover {
    background: var(--fill-1);
  }

  .chat.active {
    background: var(--sidebar-highlight, var(--fill-2));
  }

  .chat:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .avatar {
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    border-radius: var(--radius-full);
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--fill-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .chat-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chat-top,
  .chat-bottom {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .chat-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .chat-time {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .chat-time.unread {
    color: var(--success);
  }

  .chat-preview {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .unread-badge {
    flex-shrink: 0;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: var(--radius-full);
    background: var(--success);
    color: var(--on-accent, #fff);
    font-size: var(--text-caption);
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
</style>
