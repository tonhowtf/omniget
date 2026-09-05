<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    studyNotificationsList,
    studyNotificationsRead,
    studyNotificationsDismiss,
    type NotificationFull,
  } from "$lib/study-bridge";

  type Props = {
    courseId: number;
    autoCollapse?: boolean;
  };

  let { courseId, autoCollapse = false }: Props = $props();
  let items = $state<NotificationFull[]>([]);
  let loading = $state(true);
  let collapsed = $state(false);

  $effect(() => {
    collapsed = autoCollapse;
  });

  async function load() {
    loading = true;
    try {
      const all = await studyNotificationsList({ unreadOnly: false });
      items = all.filter((n) => n.course_id === courseId && n.dismissed_at == null);
    } catch (e) {
      console.error("notifications load failed", e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void courseId;
    void load();
  });

  $effect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        if (cancelled) return;
        unlisten = await listen("study:library:state-changed", () => {
          void load();
        });
        if (cancelled) {
          unlisten?.();
          unlisten = null;
        }
      } catch {
        void 0;
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  });

  async function markRead(id: number) {
    try {
      await studyNotificationsRead([id]);
      items = items.map((n) => (n.id === id ? { ...n, read_at: Date.now() } : n));
    } catch (e) {
      console.error("mark read failed", e);
    }
  }

  async function dismiss(id: number) {
    const before = items;
    items = items.filter((n) => n.id !== id);
    try {
      await studyNotificationsDismiss(id);
    } catch (e) {
      console.error("dismiss failed", e);
      items = before;
    }
  }

  async function dismissAll() {
    const ids = items.map((n) => n.id);
    items = [];
    for (const id of ids) {
      try {
        await studyNotificationsDismiss(id);
      } catch (e) {
        console.error("dismiss failed", e);
      }
    }
  }

  const unreadCount = $derived(items.filter((n) => n.read_at == null).length);
</script>

{#if !loading && items.length > 0}
  <section class="panel" aria-label={$t("notifications.inline_section_aria")}>
    <button
      type="button"
      class="panel-head"
      onclick={() => (collapsed = !collapsed)}
      aria-expanded={!collapsed}
    >
      <div class="head-left">
        <span class="bell" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
            <path d="M10 21a2 2 0 0 0 4 0" />
          </svg>
        </span>
        <span class="title">
          {items.length === 1
            ? $t("notifications.inline_count_one", { n: String(items.length) })
            : $t("notifications.inline_count_other", { n: String(items.length) })}
          {#if unreadCount > 0 && unreadCount !== items.length}
            <span class="unread-tag">{unreadCount === 1
              ? $t("notifications.inline_unread_tag_one", { n: String(unreadCount) })
              : $t("notifications.inline_unread_tag_other", { n: String(unreadCount) })}</span>
          {/if}
        </span>
      </div>
      <span class="chev" class:open={!collapsed} aria-hidden="true">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </span>
    </button>
    {#if !collapsed}
      <ul class="list">
        {#each items as n (n.id)}
          {@const isUnread = n.read_at == null}
          <li class="row" class:unread={isUnread}>
            <a
              class="row-link"
              href={n.lesson_id != null
                ? `/study/course/${n.course_id}/lesson/${n.lesson_id}`
                : `/study/course/${n.course_id}`}
              onclick={() => markRead(n.id)}
            >
              {#if isUnread}
                <span class="dot" aria-label={$t("notifications.unread_indicator")}></span>
              {/if}
              <span class="lesson-title">
                {n.lesson_title ?? $t("notifications.new_lesson_fallback")}
              </span>
            </a>
            <button
              type="button"
              class="dismiss"
              onclick={() => dismiss(n.id)}
              aria-label={$t("notifications.dismiss")}
            >×</button>
          </li>
        {/each}
      </ul>
      {#if items.length > 1}
        <button type="button" class="dismiss-all" onclick={dismissAll}>
          {$t("notifications.inline_dismiss_all")}
        </button>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .panel {
    border: 1px solid color-mix(in oklab, var(--accent) 30%, var(--content-border) 70%);
    border-radius: 10px;
    background: color-mix(in oklab, var(--accent) 4%, var(--content-bg) 96%);
    margin-bottom: 16px;
    overflow: hidden;
  }

  .panel-head {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: transparent;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 13px;
    text-align: left;
  }

  .panel-head:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }

  .head-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .bell {
    color: var(--accent);
    display: grid;
    place-items: center;
  }

  .title {
    font-weight: 500;
  }

  .unread-tag {
    margin-left: 6px;
    font-size: 11px;
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    padding: 1px 6px;
    border-radius: 10px;
  }

  .chev {
    transition: transform var(--tg-duration-fast, 150ms);
    color: color-mix(in oklab, currentColor 60%, transparent);
  }

  .chev.open {
    transform: rotate(180deg);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 4px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 6px;
  }

  .row:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }

  .row-link {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    color: inherit;
    text-decoration: none;
    font-size: 13px;
  }

  .row.unread .lesson-title {
    font-weight: 600;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    flex: 0 0 auto;
  }

  .dismiss {
    width: 24px;
    height: 24px;
    background: transparent;
    border: none;
    color: color-mix(in oklab, currentColor 50%, transparent);
    font-size: 18px;
    line-height: 1;
    border-radius: 4px;
    cursor: pointer;
    margin-right: 4px;
  }

  .dismiss:hover {
    color: var(--error);
    background: color-mix(in oklab, var(--error) 8%, transparent);
  }

  .dismiss-all {
    margin: 0 12px 10px;
    padding: 6px 10px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: color-mix(in oklab, currentColor 70%, transparent);
    font-size: 12px;
    cursor: pointer;
  }

  .dismiss-all:hover {
    color: var(--accent);
  }
</style>
