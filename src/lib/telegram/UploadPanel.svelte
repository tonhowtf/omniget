<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgIcon from "./TgIcon.svelte";
  import ChatSelect from "./ChatSelect.svelte";
  import { tg2, type TelegramChat } from "./api";
  import { failText, uploads } from "./session.svelte";

  let {
    chat = $bindable(null),
    fixedChat = false,
    onsent,
  }: { chat?: TelegramChat | null; fixedChat?: boolean; onsent?: () => void } = $props();

  let paths = $state<string[]>([]);
  let caption = $state("");
  let asDocument = $state(false);
  let groupAlbums = $state(true);
  let deleteAfter = $state(false);
  let dropping = $state(false);
  let confirming = $state(false);
  let sending = $state(false);
  let error = $state<string | null>(null);

  let uploadList = $derived(Object.values(uploads).reverse());
  let activeUploads = $derived(uploadList.filter((u) => !u.done).length);

  function add(list: string[]) {
    const seen = new Set(paths);
    paths = [...paths, ...list.filter((p) => p && !seen.has(p))];
    confirming = false;
  }

  async function pickFiles() {
    try {
      const r = await open({ multiple: true, directory: false });
      if (Array.isArray(r)) add(r);
      else if (typeof r === "string") add([r]);
    } catch {
    }
  }

  async function pickFolders() {
    try {
      const r = await open({ multiple: true, directory: true });
      if (Array.isArray(r)) add(r);
      else if (typeof r === "string") add([r]);
    } catch {
    }
  }

  function removePath(p: string) {
    paths = paths.filter((x) => x !== p);
    confirming = false;
  }

  function baseName(p: string): string {
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? p;
  }

  function looksLikeFolder(p: string): boolean {
    return !/\.[a-z0-9]{1,6}$/i.test(baseName(p));
  }

  async function send() {
    if (!chat || !paths.length) return;
    sending = true;
    error = null;
    try {
      const r = await tg2.uploadPaths(chat, {
        paths: [...paths],
        caption: caption.trim() || undefined,
        asDocument,
        deleteAfter,
        groupAlbums,
      });
      showToast("success", $t("telegram.upload.started", { count: r?.count ?? paths.length, chat: chat.title }) as string);
      paths = [];
      caption = "";
      confirming = false;
      onsent?.();
    } catch (e) {
      error = failText(e);
    } finally {
      sending = false;
    }
  }

  onMount(() => {
    let off: (() => void) | null = null;
    let disposed = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dropping = true;
          else if (p.type === "leave") dropping = false;
          else if (p.type === "drop") {
            dropping = false;
            if (p.paths?.length) add(p.paths);
          }
        })
        .then((fn) => {
          if (disposed) fn();
          else off = fn;
        })
        .catch(() => {});
    } catch {
    }
    return () => {
      disposed = true;
      off?.();
    };
  });
</script>

<div class="up">
  {#if !fixedChat}
    <div class="field">
      <span class="field-label">{$t("telegram.upload.to_chat")}</span>
      <ChatSelect bind:value={chat} label={$t("telegram.upload.to_chat")} />
    </div>
  {/if}

  <div class="drop" class:dropping class:has={paths.length > 0}>
    {#if paths.length}
      <ul class="paths">
        {#each paths as p (p)}
          <li>
            <TgIcon name={looksLikeFolder(p) ? "folder" : "file"} size={14} />
            <span class="p-name" title={p}>{baseName(p)}</span>
            <span class="p-dir" title={p}>{p}</span>
            <button class="btn btn-ghost btn-sm" onclick={() => removePath(p)} aria-label={$t("telegram.upload.remove")}><TgIcon name="x" /></button>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="drop-empty">
        <TgIcon name="upload" size={28} />
        <strong>{$t("telegram.upload.drop_title")}</strong>
        <span>{$t("telegram.upload.drop_hint")}</span>
      </div>
    {/if}
    <div class="btn-row">
      <button class="btn btn-secondary btn-sm" onclick={pickFiles}><TgIcon name="file" /> {$t("telegram.upload.add_files")}</button>
      <button class="btn btn-secondary btn-sm" onclick={pickFolders}><TgIcon name="folder" /> {$t("telegram.upload.add_folders")}</button>
      {#if paths.length}<button class="btn btn-ghost btn-sm" onclick={() => { paths = []; confirming = false; }}>{$t("telegram.browser.clear")}</button>{/if}
    </div>
  </div>

  <label class="field">
    <span class="field-label">{$t("telegram.upload.caption")}</span>
    <textarea class="input" rows="2" bind:value={caption} placeholder={$t("telegram.upload.caption_hint")}></textarea>
  </label>

  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.upload.as_document")}</span>
        <span class="group-row-sub">{$t("telegram.upload.as_document_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={asDocument} aria-label={$t("telegram.upload.as_document")} onclick={() => (asDocument = !asDocument)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.upload.group_albums")}</span>
        <span class="group-row-sub">{$t("telegram.upload.group_albums_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={groupAlbums} aria-label={$t("telegram.upload.group_albums")} onclick={() => (groupAlbums = !groupAlbums)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.upload.delete_after")}</span>
        <span class="group-row-sub">{$t("telegram.upload.delete_after_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={deleteAfter} aria-label={$t("telegram.upload.delete_after")} onclick={() => (deleteAfter = !deleteAfter)}><span class="toggle-knob"></span></button>
    </div>
  </div>

  {#if confirming && chat}
    <div class="notice notice-warning confirm">
      <TgIcon name="warn" size={16} />
      <div class="notice-body">
        <div class="notice-title">{$t("telegram.upload.confirm_title", { count: paths.length, chat: chat.title })}</div>
        <div class="notice-text">
          {$t("telegram.upload.confirm_text")}
          {#if deleteAfter}<strong> {$t("telegram.upload.confirm_delete")}</strong>{/if}
        </div>
      </div>
    </div>
  {/if}

  {#if error}<p class="field-error">{error}</p>{/if}

  <div class="btn-row end">
    {#if confirming}
      <button class="btn btn-secondary" onclick={() => (confirming = false)} disabled={sending}>{$t("telegram.common.back")}</button>
      <button class="btn btn-primary" onclick={send} disabled={sending}>
        {#if sending}<span class="spinner"></span>{:else}<TgIcon name="share" />{/if}
        {$t("telegram.upload.send_now")}
      </button>
    {:else}
      <button class="btn btn-primary" onclick={() => (confirming = true)} disabled={!chat || !paths.length}>
        <TgIcon name="upload" /> {$t("telegram.upload.review")}
      </button>
    {/if}
  </div>

  {#if uploadList.length}
    <div class="uploads">
      <div class="u-head">{$t("telegram.upload.progress_title")}{#if activeUploads} · {$t("telegram.upload.active", { count: activeUploads })}{/if}</div>
      {#each uploadList.slice(0, 30) as u (u.id)}
        <div class="u-row">
          <span class="u-name" title={u.title}>{u.title}</span>
          {#if u.error}
            <span class="u-err" title={u.error}>{$t("telegram.upload.failed")}</span>
          {:else if u.done}
            <span class="u-ok"><TgIcon name="check" size={12} /></span>
          {:else}
            <span class="u-pct">{Math.round(u.percent)}%</span>
          {/if}
          <div class="progress"><div class="progress-fill" style:width="{u.percent}%"></div></div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .up {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  textarea.input {
    resize: vertical;
    min-height: 56px;
    font: inherit;
  }

  .drop {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    border: 1.5px dashed var(--separator);
    background: var(--fill-1);
    transition: background var(--duration-fast) ease, border-color var(--duration-fast) ease;
  }

  .drop.dropping {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .drop-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: var(--space-4) 0 var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
    text-align: center;
  }

  .drop-empty strong {
    color: var(--text);
    font-size: var(--text-base);
  }

  .drop .btn-row {
    justify-content: center;
  }

  .drop.has .btn-row {
    justify-content: flex-start;
  }

  .paths {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 200px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .paths li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .p-name {
    flex-shrink: 0;
    max-width: 50%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-weight: 550;
  }

  .p-dir {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .confirm strong {
    color: var(--danger);
  }

  .uploads {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .u-head {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .u-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 120px;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .u-name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text);
  }

  .u-pct {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .u-ok {
    color: var(--success);
  }

  .u-err {
    color: var(--danger);
    font-size: var(--text-xs);
  }
</style>
