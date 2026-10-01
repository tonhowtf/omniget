<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import ChatSelect from "./ChatSelect.svelte";
  import { tg2, type CloneProgress, type CloneSession, type TelegramChat } from "./api";
  import { chatRef, failText } from "./session.svelte";

  let { source = $bindable(null) }: { source?: TelegramChat | null } = $props();

  let sessions = $state<CloneSession[]>([]);
  let loaded = $state(false);
  let listError = $state<string | null>(null);
  let destMode = $state<"new" | "existing">("new");
  let dest = $state<TelegramChat | null>(null);
  let destTitle = $state("");
  let delayMs = $state(2000);
  let batchSize = $state(50);
  let limitOn = $state(false);
  let limit = $state(100);
  let dropAuthor = $state(false);
  let dropCaptions = $state(false);
  let confirming = $state(false);
  let starting = $state(false);
  let error = $state<string | null>(null);

  const notPrivate = (c: TelegramChat) => c.chat_type !== "private";

  async function refresh() {
    try {
      sessions = (await tg2.cloneList()) ?? [];
      listError = null;
    } catch (e) {
      listError = failText(e);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    void refresh();
    let off: UnlistenFn | null = null;
    let disposed = false;
    listen<CloneProgress>("telegram-clone-progress", (ev) => {
      const p = ev.payload;
      const s = sessions.find((x) => x.id === p.session_id);
      if (!s) {
        void refresh();
        return;
      }
      s.total_collected = p.total;
      s.cloned_count = p.current;
      s.failed_count = p.failed;
      s.last_message_id = p.last_message_id;
      if (p.stage === "completed" || p.stage === "paused" || p.stage === "error" || p.stage === "cancelled") s.status = p.stage;
      else s.status = "running";
      if (p.error) s.error = p.error;
    })
      .then((fn) => {
        if (disposed) fn();
        else off = fn;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      off?.();
    };
  });

  function options() {
    return {
      delay_ms: Math.max(0, Math.floor(delayMs)),
      batch_size: Math.max(1, Math.min(100, Math.floor(batchSize))),
      limit: limitOn ? Math.max(1, Math.floor(limit)) : null,
      drop_author: dropAuthor,
      drop_captions: dropCaptions,
    };
  }

  async function start() {
    if (!source) return;
    starting = true;
    error = null;
    try {
      const r = await tg2.cloneStart({
        source,
        dest: destMode === "existing" ? dest : null,
        destTitle: destMode === "new" ? destTitle.trim() : undefined,
        options: options(),
      });
      showToast("info", $t("telegram.clone.started", { title: r.dest_title }) as string);
      confirming = false;
      await refresh();
    } catch (e) {
      error = failText(e);
      confirming = false;
    } finally {
      starting = false;
    }
  }

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      showToast("error", failText(e));
    }
    await refresh();
  }

  function resume(s: CloneSession) {
    return act(() =>
      tg2.cloneStart({
        source: chatRef(s.source_chat_id, s.source_chat_type, s.source_title),
        dest: chatRef(s.dest_chat_id, s.dest_chat_type, s.dest_title),
        resumeId: s.id,
        options: { ...s.options, limit: s.options.limit ?? null },
      }),
    );
  }

  function pct(s: CloneSession): number {
    return s.total_collected ? Math.min(100, (s.cloned_count / s.total_collected) * 100) : 0;
  }

  let canStart = $derived(!!source && (destMode === "new" || !!dest));
</script>

<section class="clone">
  <header>
    <h3>{$t("telegram.clone.title")}</h3>
    <p>{$t("telegram.clone.hint")}</p>
  </header>

  <div class="group">
    <div class="group-row stack">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.clone.source")}</span></div>
      <ChatSelect bind:value={source} filter={notPrivate} label={$t("telegram.clone.source")} />
    </div>
    <div class="group-row stack">
      <div class="group-row-content"><span class="group-row-title">{$t("telegram.clone.dest")}</span></div>
      <div class="segmented" role="radiogroup" aria-label={$t("telegram.clone.dest")}>
        <button class="segmented-btn" role="radio" aria-checked={destMode === "new"} onclick={() => (destMode = "new")}>{$t("telegram.clone.dest_new")}</button>
        <button class="segmented-btn" role="radio" aria-checked={destMode === "existing"} onclick={() => (destMode = "existing")}>{$t("telegram.clone.dest_existing")}</button>
      </div>
      {#if destMode === "new"}
        <input class="input" bind:value={destTitle} placeholder={source ? `${source.title} - clone` : $t("telegram.clone.dest_name")} aria-label={$t("telegram.clone.dest_name")} />
        <span class="field-hint">{$t("telegram.clone.dest_new_hint")}</span>
      {:else}
        <ChatSelect bind:value={dest} exclude={source} filter={notPrivate} label={$t("telegram.clone.dest")} />
        <span class="field-hint">{$t("telegram.clone.dest_existing_hint")}</span>
      {/if}
    </div>
  </div>

  <details class="adv">
    <summary>{$t("telegram.clone.advanced")}</summary>
    <div class="adv-grid">
      <label class="field">
        <span class="field-label">{$t("telegram.clone.delay")}</span>
        <input class="input" type="number" min="0" max="60000" step="250" bind:value={delayMs} />
      </label>
      <label class="field">
        <span class="field-label">{$t("telegram.clone.batch")}</span>
        <input class="input" type="number" min="1" max="100" bind:value={batchSize} />
      </label>
      <label class="check"><input type="checkbox" bind:checked={limitOn} /> {$t("telegram.clone.limit")}</label>
      {#if limitOn}<input class="input" type="number" min="1" bind:value={limit} aria-label={$t("telegram.clone.limit")} />{/if}
      <label class="check"><input type="checkbox" bind:checked={dropAuthor} /> {$t("telegram.forward.drop_author")}</label>
      <label class="check"><input type="checkbox" bind:checked={dropCaptions} /> {$t("telegram.forward.drop_captions")}</label>
    </div>
  </details>

  {#if confirming && source}
    <div class="notice notice-warning">
      <TgIcon name="warn" size={16} />
      <div class="notice-body">
        <div class="notice-title">
          {$t("telegram.clone.confirm", { source: source.title, dest: destMode === "existing" ? (dest?.title ?? "") : destTitle.trim() || `${source.title} - clone` })}
        </div>
        <div class="notice-text">{$t("telegram.clone.confirm_text")}</div>
      </div>
    </div>
  {/if}
  {#if error}<p class="field-error">{error}</p>{/if}

  <div class="btn-row end">
    {#if confirming}
      <button class="btn btn-secondary" onclick={() => (confirming = false)} disabled={starting}>{$t("telegram.common.back")}</button>
      <button class="btn btn-primary" onclick={start} disabled={starting}>
        {#if starting}<span class="spinner"></span>{:else}<TgIcon name="clone" />{/if}
        {$t("telegram.clone.start_now")}
      </button>
    {:else}
      <button class="btn btn-primary" onclick={() => (confirming = true)} disabled={!canStart}><TgIcon name="clone" /> {$t("telegram.clone.review")}</button>
    {/if}
  </div>

  <div class="sessions">
    <div class="s-head">
      <h4>{$t("telegram.clone.sessions")}</h4>
      <button class="btn btn-ghost btn-sm" onclick={() => void refresh()} aria-label={$t("telegram.common.retry")}><TgIcon name="refresh" /></button>
    </div>
    {#if listError}
      <p class="field-error">{listError}</p>
    {:else if !loaded}
      <p class="dim"><span class="spinner"></span> {$t("telegram.common.loading")}</p>
    {:else if !sessions.length}
      <p class="dim">{$t("telegram.clone.no_sessions")}</p>
    {:else}
      {#each sessions as s (s.id)}
        <div class="sess">
          <div class="s-top">
            <span class="s-src">{s.source_title}</span>
            <TgIcon name="chevron" size={12} />
            <span class="s-dst">{s.dest_title}</span>
            <span class="tag" class:tag-success={s.status === "completed"} class:tag-danger={s.status === "error"} class:tag-accent={s.status === "running"}>
              {$t(`telegram.clone.status.${["running", "paused", "completed", "error", "cancelled"].includes(s.status) ? s.status : "paused"}`)}
            </span>
          </div>
          <div class="progress"><div class="progress-fill" style:width="{pct(s)}%"></div></div>
          <div class="s-meta">
            {$t("telegram.clone.counts", { done: s.cloned_count, total: s.total_collected || "?" })}
            {#if s.failed_count}· {$t("telegram.browser.batch_failed", { count: s.failed_count })}{/if}
          </div>
          {#if s.error}<p class="field-error">{s.error}</p>{/if}
          <div class="btn-row">
            {#if s.status === "running"}
              <button class="btn btn-secondary btn-sm" onclick={() => act(() => tg2.clonePause(s.id))}>{$t("telegram.clone.pause")}</button>
            {:else if s.status === "paused" || s.status === "error"}
              <button class="btn btn-primary btn-sm" onclick={() => resume(s)}>{$t("telegram.clone.resume")}</button>
            {/if}
            {#if s.status === "running" || s.status === "paused"}
              <button class="btn btn-ghost btn-sm" onclick={() => act(() => tg2.cloneCancel(s.id))}>{$t("telegram.common.cancel")}</button>
            {:else}
              <button class="btn btn-ghost btn-sm" onclick={() => act(() => tg2.cloneDelete(s.id))}><TgIcon name="trash" /> {$t("telegram.clone.delete")}</button>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>
</section>

<style>
  .clone {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 680px;
  }

  header h3,
  .s-head h4 {
    margin: 0 0 2px;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .s-head h4 {
    font-size: var(--text-base);
    margin: 0;
  }

  header p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .group-row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }

  .group-row.stack .segmented {
    align-self: flex-start;
  }

  .adv summary {
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: 550;
    color: var(--text-muted);
  }

  .adv-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
    margin-top: var(--space-2);
    align-items: center;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .sessions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .s-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .sess {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .s-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .s-src,
  .s-dst {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-weight: 550;
  }

  .s-top .tag {
    margin-left: auto;
  }

  .s-meta,
  .dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
