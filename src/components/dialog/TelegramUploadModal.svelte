<script lang="ts">
  import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import {
    telegramUploadFile,
    calculateTelegramChunks,
    translateTelegramError,
    type TelegramUploadTier,
    type TelegramSendAsMode,
    type TelegramChat,
    telegramGetChats,
  } from "$lib/study-telegram-bridge";
  import { onMount } from "svelte";

  let {
    open = false,
    filePath = "",
    fileSize = 0,
    defaultName = "",
    defaultThumb = null as string | null,
    onClose = () => {},
  }: {
    open?: boolean;
    filePath?: string;
    fileSize?: number;
    defaultName?: string;
    defaultThumb?: string | null;
    onClose?: () => void;
  } = $props();

  type AuthMode = "user_session" | "bot_token";
  let authMode = $state<AuthMode>("user_session");

  let botToken = $state("");
  let destChatId = $state("");
  let customFileName = $state("");
  let customThumbPath = $state<string | null>(null);
  let caption = $state("");
  let tier = $state<TelegramUploadTier>("free");
  let sendAs = $state<TelegramSendAsMode>("video");

  let chats = $state<TelegramChat[]>([]);
  let loadingChats = $state(false);
  let selectedChatId = $state<number | null>(null);

  let uploading = $state(false);
  let uploadSuccess = $state(false);
  let uploadError = $state<string | null>(null);

  function autoDetectSendAs(fileNameOrPath: string): TelegramSendAsMode {
    const ext = fileNameOrPath.split(".").pop()?.toLowerCase() ?? "";
    if (["mp4", "mkv", "avi", "mov", "webm", "flv", "m4v"].includes(ext)) return "video";
    if (["mp3", "m4a", "flac", "aac", "ogg", "wav", "opus"].includes(ext)) return "audio";
    return "document";
  }

  function onKeydown(e: KeyboardEvent) {
    if (open && e.key === "Escape" && !uploading) {
      e.preventDefault();
      onClose();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  $effect(() => {
    if (open) {
      customFileName = defaultName || filePath.split(/[/\\]/).pop() || "";
      customThumbPath = defaultThumb || null;
      sendAs = autoDetectSendAs(customFileName || filePath);
      uploading = false;
      uploadSuccess = false;
      uploadError = null;
      if (authMode === "user_session" && chats.length === 0) {
        loadUserChats();
      }
    }
  });

  async function loadUserChats() {
    loadingChats = true;
    try {
      chats = await telegramGetChats();
      if (chats.length > 0) {
        selectedChatId = chats[0].id;
      }
    } catch {
      // User session might not be logged in
    } finally {
      loadingChats = false;
    }
  }

  let chunkAnalysis = $derived(calculateTelegramChunks(fileSize, tier));

  async function pickThumbnail() {
    try {
      const selected = await openFileDialog({
        title: $t("telegram.pick_thumbnail") as string,
        filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }],
        multiple: false,
      });
      if (selected && typeof selected === "string") {
        customThumbPath = selected;
      }
    } catch (e) {
      console.warn("pick thumbnail error", e);
    }
  }

  function clearThumbnail() {
    customThumbPath = null;
  }

  function normalizeTargetChat(input: string): string {
    let trimmed = input.trim();
    if (trimmed.startsWith("https://t.me/")) {
      const parts = trimmed.replace("https://t.me/", "").split("/");
      return "@" + parts[0];
    }
    if (trimmed.startsWith("t.me/")) {
      const parts = trimmed.replace("t.me/", "").split("/");
      return "@" + parts[0];
    }
    return trimmed;
  }

  function validateBotToken(token: string): boolean {
    return /^\d+:[A-Za-z0-9_-]+$/.test(token.trim());
  }

  async function handleUpload() {
    if (!filePath.trim()) {
      uploadError = $t("telegram.error_no_file") as string;
      return;
    }

    const rawTarget = authMode === "bot_token"
      ? destChatId
      : (selectedChatId ? String(selectedChatId) : destChatId);

    const targetChat = normalizeTargetChat(rawTarget);

    if (!targetChat) {
      uploadError = $t("telegram.error_no_chat") as string;
      return;
    }
    if (authMode === "bot_token") {
      if (!botToken.trim()) {
        uploadError = $t("telegram.error_no_bot_token") as string;
        return;
      }
      if (!validateBotToken(botToken)) {
        uploadError = $t("telegram.error_invalid_bot_token") as string;
        return;
      }
    }

    uploading = true;
    uploadError = null;
    uploadSuccess = false;

    try {
      const res = await telegramUploadFile({
        filePath,
        destChatId: targetChat,
        customFileName: customFileName.trim() || undefined,
        customThumbnailPath: customThumbPath || undefined,
        caption: caption.trim() || undefined,
        botToken: authMode === "bot_token" ? botToken.trim() : undefined,
        tier,
        sendAs,
      });

      if (res.success || res.messageId) {
        uploadSuccess = true;
        showToast("success", $t("telegram.upload_success") as string);
        setTimeout(() => {
          onClose();
        }, 1200);
      } else {
        uploadError = $t("telegram.upload_failed") as string;
      }
    } catch (e: any) {
      const rawErr = typeof e === "string" ? e : e.message ?? ($t("telegram.upload_failed") as string);
      uploadError = translateTelegramError(rawErr, (key) => $t(key) as string);
      showToast("error", uploadError);
    } finally {
      uploading = false;
    }
  }
</script>

{#if open}
  <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !uploading) onClose(); }}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="tg-upload-title">
      <header class="head">
        <h3 id="tg-upload-title">{$t('telegram.upload_title')}</h3>
        <button class="close" onclick={onClose} disabled={uploading} aria-label={$t('common.close')}>×</button>
      </header>

      <div class="body">
        <div class="auth-tabs" role="tablist">
          <button
            class="tab-btn"
            class:active={authMode === "user_session"}
            onclick={() => { authMode = "user_session"; if (chats.length === 0) loadUserChats(); }}
            role="tab"
            aria-selected={authMode === "user_session"}
          >
            {$t('telegram.auth_user_session')}
          </button>
          <button
            class="tab-btn"
            class:active={authMode === "bot_token"}
            onclick={() => { authMode = "bot_token"; }}
            role="tab"
            aria-selected={authMode === "bot_token"}
          >
            {$t('telegram.auth_bot_token')}
          </button>
        </div>

        {#if authMode === "user_session"}
          <div class="field-group">
            <label class="field-label" for="tg-chat-select">{$t('telegram.destination_chat')}</label>
            {#if loadingChats}
              <div class="field-hint">{$t('common.loading')}</div>
            {:else if chats.length > 0}
              <select id="tg-chat-select" class="select-full" bind:value={selectedChatId}>
                {#each chats as chat (chat.id)}
                  <option value={chat.id}>{chat.title} ({chat.chat_type})</option>
                {/each}
              </select>
            {:else}
              <input
                id="tg-chat-select"
                type="text"
                class="input-full"
                placeholder={$t('telegram.dest_placeholder')}
                bind:value={destChatId}
              />
              <span class="field-hint">{$t('telegram.user_session_hint')}</span>
            {/if}
          </div>
        {:else}
          <div class="field-group">
            <label class="field-label" for="tg-bot-token">{$t('telegram.bot_token_label')}</label>
            <input
              id="tg-bot-token"
              type="password"
              class="input-full"
              placeholder="123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ"
              bind:value={botToken}
            />
          </div>
          <div class="field-group">
            <label class="field-label" for="tg-bot-chat">{$t('telegram.bot_chat_id_label')}</label>
            <input
              id="tg-bot-chat"
              type="text"
              class="input-full"
              placeholder="@channel_username, chat_id or -100123456789"
              bind:value={destChatId}
            />
          </div>
        {/if}

        <div class="field-group" role="radiogroup" aria-label={$t('telegram.send_as_label') as string}>
          <span class="field-label">{$t('telegram.send_as_label')}</span>
          <div class="send-as-pills">
            <button
              class="send-as-pill"
              class:active={sendAs === "video"}
              onclick={() => { sendAs = "video"; }}
            >
              🎬 {$t('telegram.send_as_video')}
            </button>
            <button
              class="send-as-pill"
              class:active={sendAs === "document"}
              onclick={() => { sendAs = "document"; }}
            >
              📄 {$t('telegram.send_as_document')}
            </button>
            <button
              class="send-as-pill"
              class:active={sendAs === "audio"}
              onclick={() => { sendAs = "audio"; }}
            >
              🎵 {$t('telegram.send_as_audio')}
            </button>
          </div>
        </div>

        <div class="field-group">
          <label class="field-label" for="tg-filename">{$t('telegram.custom_filename_label')}</label>
          <input
            id="tg-filename"
            type="text"
            class="input-full"
            bind:value={customFileName}
            placeholder="video_name.mp4"
          />
          <span class="field-hint">{$t('telegram.custom_filename_hint')}</span>
        </div>

        <div class="field-group">
          <label class="field-label" for="tg-thumb-btn">{$t('telegram.custom_thumbnail_label')}</label>
          <div class="thumb-row">
            {#if customThumbPath}
              <span class="thumb-path" title={customThumbPath}>{customThumbPath.split(/[/\\]/).pop()}</span>
              <button class="button button-small" onclick={clearThumbnail}>{$t('common.remove')}</button>
            {:else}
              <button id="tg-thumb-btn" class="button" onclick={pickThumbnail}>{$t('telegram.pick_thumbnail')}</button>
            {/if}
          </div>
        </div>

        <div class="field-group">
          <label class="field-label" for="tg-caption">{$t('telegram.caption_label')}</label>
          <textarea
            id="tg-caption"
            class="textarea-full"
            rows="2"
            bind:value={caption}
            placeholder={$t('telegram.caption_placeholder') as string}
          ></textarea>
        </div>

        <div class="field-group" role="radiogroup" aria-label={$t('telegram.tier_limit_label') as string}>
          <span class="field-label">{$t('telegram.tier_limit_label')}</span>
          <div class="tier-pills">
            <button
              class="tier-pill"
              class:active={tier === "free"}
              onclick={() => { tier = "free"; }}
            >
              {$t('telegram.tier_free')} (2.0 GB)
            </button>
            <button
              class="tier-pill"
              class:active={tier === "premium"}
              onclick={() => { tier = "premium"; }}
            >
              {$t('telegram.tier_premium')} (4.0 GB)
            </button>
          </div>
        </div>

        {#if chunkAnalysis.requiresSplit}
          <div class="split-warning">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
            <span>
              {$t('telegram.split_warning', {
                size: formatBytes(fileSize),
                limit: formatBytes(chunkAnalysis.limitBytes),
                parts: String(chunkAnalysis.partCount),
              })}
            </span>
          </div>
        {/if}

        {#if uploadError}
          <div class="error-banner">{uploadError}</div>
        {/if}
      </div>

      <footer class="foot">
        <button class="button" onclick={onClose} disabled={uploading}>{$t('common.cancel')}</button>
        <button class="button action-btn" onclick={handleUpload} disabled={uploading}>
          {#if uploading}
            <span class="spinner"></span> {$t('telegram.uploading')}
          {:else if uploadSuccess}
            ✓ {$t('telegram.upload_success')}
          {:else}
            {$t('telegram.start_upload')}
          {/if}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.5));
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .dialog {
    background: var(--popup-bg, #1a1a1a);
    border: 1px solid var(--content-border, rgba(255, 255, 255, 0.1));
    border-radius: var(--border-radius, 12px);
    width: 490px;
    max-width: 90vw;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--content-border);
  }

  .head h3 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
  }

  .close {
    background: none;
    border: none;
    font-size: 20px;
    color: var(--gray);
    cursor: pointer;
  }

  .body {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: 70vh;
    overflow-y: auto;
  }

  .auth-tabs {
    display: flex;
    gap: 4px;
    background: var(--button);
    padding: 3px;
    border-radius: 8px;
  }

  .tab-btn {
    flex: 1;
    padding: 6px 10px;
    font-size: 12px;
    font-weight: 500;
    color: var(--gray);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
  }

  .tab-btn.active {
    background: var(--button-elevated);
    color: var(--text);
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .field-hint {
    font-size: 11px;
    color: var(--gray);
    opacity: 0.8;
  }

  .input-full,
  .select-full,
  .textarea-full {
    width: 100%;
    padding: 8px 12px;
    font-size: 13px;
    background: var(--input-bg, rgba(0, 0, 0, 0.2));
    border: 1px solid var(--input-border, rgba(255, 255, 255, 0.15));
    border-radius: 8px;
    color: var(--text);
    box-sizing: border-box;
  }

  .send-as-pills,
  .tier-pills {
    display: flex;
    gap: 8px;
  }

  .send-as-pill,
  .tier-pill {
    flex: 1;
    padding: 6px 10px;
    font-size: 12px;
    font-weight: 500;
    color: var(--gray);
    background: var(--button);
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: pointer;
    text-align: center;
  }

  .send-as-pill.active,
  .tier-pill.active {
    background: var(--button-elevated);
    color: var(--accent);
    border-color: var(--accent);
  }

  .thumb-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .thumb-path {
    font-size: 12px;
    font-family: var(--font-mono);
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 250px;
  }

  .split-warning {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    background: color-mix(in srgb, #f59e0b 15%, transparent);
    color: #f59e0b;
    border-radius: 8px;
    font-size: 12px;
    line-height: 1.4;
  }

  .error-banner {
    padding: 8px 12px;
    background: color-mix(in srgb, var(--error, #ef4444) 15%, transparent);
    color: var(--error, #ef4444);
    border-radius: 8px;
    font-size: 12px;
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    padding: 12px 18px;
    border-top: 1px solid var(--content-border);
  }

  .action-btn {
    background: var(--accent);
    color: #fff;
    border: none;
    font-weight: 600;
  }

  .button-small {
    padding: 4px 8px;
    font-size: 11px;
  }

  .spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-radius: 50%;
    border-top-color: #fff;
    animation: spin 0.8s linear infinite;
  }

  .input-full:focus-visible,
  .select-full:focus-visible,
  .textarea-full:focus-visible,
  .tab-btn:focus-visible,
  .send-as-pill:focus-visible,
  .tier-pill:focus-visible,
  .button:focus-visible,
  .close:focus-visible {
    outline: 2px solid var(--accent, #3b82f6);
    outline-offset: 2px;
  }

  @media (max-width: 535px) {
    .dialog {
      width: 100vw;
      max-width: 100vw;
      border-radius: 0;
      height: 100vh;
      max-height: 100vh;
    }
    .send-as-pills,
    .tier-pills {
      flex-direction: column;
    }
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
