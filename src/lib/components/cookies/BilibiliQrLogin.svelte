<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";

  type QrSession = {
    qrcode_key: string;
    login_url: string;
    qrcode_svg: string;
  };

  type QrPollStatus =
    | { status: "pending" }
    | { status: "scanned" }
    | { status: "success"; slug: string; uname: string; mid: number; is_vip: boolean }
    | { status: "expired" }
    | { status: "cancelled" };

  type Props = {
    open: boolean;
    onClose: () => void;
    onSuccess: (slug: string, uname: string) => void;
  };

  let { open, onClose, onSuccess }: Props = $props();

  let session = $state<QrSession | null>(null);
  let polling = $state(false);
  let phase = $state<"loading" | "pending" | "scanned" | "expired" | "error">("loading");
  let errorMessage = $state("");
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let cancelled = false;

  async function start() {
    cancelled = false;
    phase = "loading";
    errorMessage = "";
    try {
      session = await invoke<QrSession>("bilibili_qr_generate", {
        userAgent: null,
      });
      phase = "pending";
      beginPolling();
    } catch (e) {
      const key = typeof e === "string" ? e : "errors.bilibili.network_failed";
      errorMessage = ($t(key) as string) || ($t("settings.cookies.bilibili.login_error_generic") as string);
      phase = "error";
    }
  }

  function beginPolling() {
    stopPolling();
    polling = true;
    pollTimer = setInterval(async () => {
      if (cancelled || !session) return;
      try {
        const result = await invoke<QrPollStatus>("bilibili_qr_poll", {
          qrcodeKey: session.qrcode_key,
          userAgent: null,
        });
        if (result.status === "pending") {
          phase = "pending";
        } else if (result.status === "scanned") {
          phase = "scanned";
        } else if (result.status === "success") {
          stopPolling();
          onSuccess(result.slug, result.uname);
        } else if (result.status === "expired") {
          stopPolling();
          phase = "expired";
        } else if (result.status === "cancelled") {
          stopPolling();
          phase = "expired";
        }
      } catch (e) {
        const key = typeof e === "string" ? e : "errors.bilibili.network_failed";
        errorMessage = ($t(key) as string) || "";
      }
    }, 2000);
  }

  function stopPolling() {
    polling = false;
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function handleClose() {
    cancelled = true;
    stopPolling();
    session = null;
    phase = "loading";
    errorMessage = "";
    onClose();
  }

  $effect(() => {
    if (open) {
      start();
    } else {
      stopPolling();
    }
  });
</script>

{#if open}
  <div
    class="overlay"
    onclick={(e) => {
      if (e.target === e.currentTarget) handleClose();
    }}
    onkeydown={(e) => {
      if (e.key === "Escape") handleClose();
    }}
    role="presentation"
  >
    <div class="modal" role="dialog" aria-label={$t("settings.cookies.bilibili.login_qr_title") as string}>
      <header class="head">
        <h2>{$t("settings.cookies.bilibili.login_qr_title")}</h2>
        <p class="subtitle">{$t("settings.cookies.bilibili.login_qr_subtitle")}</p>
        <button
          type="button"
          class="close"
          onclick={handleClose}
          aria-label={$t("settings.cookies.bilibili.login_close") as string}>×</button
        >
      </header>

      <div class="body">
        {#if phase === "loading"}
          <div class="loading-state">
            <div class="spinner" aria-hidden="true"></div>
          </div>
        {:else if phase === "error"}
          <div class="error-state">
            <p class="err-msg">{errorMessage}</p>
            <button type="button" class="primary-btn" onclick={start}>
              {$t("settings.cookies.bilibili.login_qr_regenerate")}
            </button>
          </div>
        {:else if phase === "expired"}
          <div class="expired-state">
            <p>{$t("settings.cookies.bilibili.login_qr_expired")}</p>
            <button type="button" class="primary-btn" onclick={start}>
              {$t("settings.cookies.bilibili.login_qr_regenerate")}
            </button>
          </div>
        {:else if session}
          <div class="qr-wrap" data-phase={phase}>
            {@html session.qrcode_svg}
          </div>
          <p class="status" data-phase={phase}>
            {#if phase === "scanned"}
              {$t("settings.cookies.bilibili.login_qr_scanned")}
            {:else}
              {$t("settings.cookies.bilibili.login_qr_pending")}
            {/if}
          </p>
        {/if}
      </div>

      <footer class="foot">
        <button type="button" class="ghost-btn" onclick={handleClose}>
          {$t("settings.cookies.bilibili.login_cancel")}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, var(--bg) 75%, black);
    display: grid;
    place-items: center;
    z-index: 1100;
    padding: 16px;
  }
  .modal {
    width: min(420px, 96vw);
    background: var(--surface);
    border: none;
    border-radius: 16px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .head {
    position: relative;
    padding: 20px 24px 16px;
    border-bottom: none;
  }
  .head h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
  }
  .subtitle {
    margin: 6px 0 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .close {
    position: absolute;
    top: 12px;
    right: 12px;
    width: 32px;
    height: 32px;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 22px;
    line-height: 1;
    cursor: pointer;
    border-radius: 8px;
  }
  .close:hover {
    background: color-mix(in oklab, var(--text) 8%, transparent);
    color: var(--text);
  }
  .body {
    padding: 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
  }
  .qr-wrap {
    width: 260px;
    height: 260px;
    padding: 12px;
    background: white;
    border-radius: 12px;
    display: grid;
    place-items: center;
    transition: opacity 200ms;
  }
  .qr-wrap[data-phase="scanned"] {
    opacity: 0.45;
  }
  .qr-wrap :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .status {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
  }
  .status[data-phase="scanned"] {
    color: var(--accent);
  }
  .loading-state,
  .expired-state,
  .error-state {
    width: 260px;
    height: 260px;
    display: grid;
    place-items: center;
    gap: 12px;
    color: var(--text-muted);
    text-align: center;
  }
  .err-msg {
    color: var(--danger);
    font-size: 13px;
    margin: 0;
  }
  .spinner {
    width: 32px;
    height: 32px;
    border: 3px solid color-mix(in oklab, var(--text) 15%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 800ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .primary-btn {
    padding: 8px 14px;
    border-radius: 8px;
    border: 0;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 13px;
    cursor: pointer;
  }
  .foot {
    padding: 12px 20px;
    border-top: none;
    display: flex;
    justify-content: flex-end;
  }
  .ghost-btn {
    padding: 8px 14px;
    border-radius: 8px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }
  .ghost-btn:hover {
    background: color-mix(in oklab, var(--text) 6%, transparent);
  }
</style>
