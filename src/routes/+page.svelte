<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import Mascot from "$components/mascot/Mascot.svelte";
  import { t } from "$lib/i18n";

  type PlatformInfo = {
    platform: string;
    supported: boolean;
  };

  let url = $state("");
  let detection = $state<PlatformInfo | null>(null);
  let detecting = $state(false);
  let debounceTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  function isUrl(value: string): boolean {
    return value.startsWith("http://") || value.startsWith("https://");
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);

    if (!url.trim() || !isUrl(url.trim())) {
      detection = null;
      detecting = false;
      return;
    }

    detecting = true;
    debounceTimer = setTimeout(() => {
      detectPlatform(url.trim());
    }, 500);
  }

  async function detectPlatform(value: string) {
    try {
      detection = await invoke<PlatformInfo>("detect_platform", { url: value });
    } catch {
      detection = null;
    } finally {
      detecting = false;
    }
  }

  function handleAction() {
    if (!detection?.supported) return;
    if (detection.platform === "hotmart") {
      goto("/hotmart");
    }
  }
</script>

<div class="home">
  <Mascot emotion="sad" />

  <div class="omnibox-area">
    <div class="omnibox-wrapper">
      <input
        class="omnibox"
        type="text"
        placeholder={$t('omnibox.placeholder')}
        bind:value={url}
        oninput={handleInput}
      />
    </div>

    {#if detecting}
      <div class="feedback">
        <span class="feedback-spinner"></span>
      </div>
    {:else if detection}
      <div class="feedback" data-supported={detection.supported}>
        {#if detection.supported}
          <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <span class="feedback-text">{$t('omnibox.detected', { platform: detection.platform.charAt(0).toUpperCase() + detection.platform.slice(1) })}</span>
        {:else}
          <svg class="feedback-icon" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4m0 4h.01" />
          </svg>
          <span class="feedback-text">{$t('omnibox.unsupported')}</span>
        {/if}
      </div>

      {#if detection.supported}
        <button class="button action-btn" onclick={handleAction}>
          {#if detection.platform === "hotmart"}
            {$t('omnibox.go_to_hotmart')}
          {:else}
            {$t('omnibox.download')}
          {/if}
        </button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 2);
  }

  .omnibox-area {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    width: 100%;
    max-width: 560px;
  }

  .omnibox-wrapper {
    width: 100%;
  }

  .omnibox {
    width: 100%;
    padding: var(--padding) calc(var(--padding) + 4px);
    font-size: 14.5px;
    background: var(--button);
    border-radius: var(--border-radius);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .omnibox::placeholder {
    color: var(--gray);
  }

  .omnibox:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }

  .feedback {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .feedback-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .feedback[data-supported="true"] {
    color: var(--green);
  }

  .feedback[data-supported="false"] {
    color: var(--gray);
  }

  .feedback-text {
    font-size: 12.5px;
    font-weight: 500;
  }

  .feedback-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .action-btn {
    padding: calc(var(--padding) / 2) calc(var(--padding) * 1.5);
    font-size: 14.5px;
  }
</style>
