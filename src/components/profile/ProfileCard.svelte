<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import Avatar from "#components/omni/Avatar.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    copyFingerprint,
    formatFingerprint,
    type Profile,
  } from "$lib/stores/profile-store.svelte";

  let {
    profile,
  }: { profile: Profile } = $props();

  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  onDestroy(() => {
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = null;
  });

  let grouped = $derived(formatFingerprint(profile.fingerprint));

  async function copy() {
    const ok = await copyFingerprint();
    if (!ok) {
      showToast("error", $t("profile.copy_failed") as string);
      return;
    }
    copied = true;
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => {
      copied = false;
      copyTimer = null;
    }, 1600);
    showToast("success", $t("profile.copied") as string);
  }
</script>

<div class="profile-card">
  <div class="profile-header">
    <Avatar tint={profile.skin?.tint} size={56} />
    <div class="profile-meta">
      <span class="profile-nick">{profile.nickname}</span>
      <span class="profile-hint">{$t("profile.used_in_hint")}</span>
    </div>
  </div>

  {#if grouped}
    <details class="fp-disclosure">
      <summary>{$t("profile.identity_disclosure")}</summary>
      <div class="fp-row">
        <code class="fp">{grouped}</code>
        <button
          type="button"
          class="btn btn-secondary btn-sm fp-copy"
          onclick={copy}
          aria-label={$t("profile.copy_fingerprint") as string}
        >
          {copied ? $t("profile.copied") : $t("profile.copy")}
        </button>
      </div>
    </details>
  {/if}
</div>

<style>
  .profile-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--surface-hi) 45%, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    min-width: 0;
  }

  .profile-header {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    min-width: 0;
  }

  .profile-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .profile-nick {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .profile-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.45;
  }

  .fp-disclosure {
    border-top: var(--hairline) solid var(--separator);
    padding-top: var(--space-2);
  }

  .fp-disclosure summary {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }

  .fp-disclosure summary:hover {
    color: var(--text-muted);
  }

  .fp-disclosure summary:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-xs);
  }

  .fp-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    min-width: 0;
  }

  .fp {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
    color: var(--text-muted);
    background: var(--surface);
    padding: 4px 8px;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }

  .fp-copy {
    flex-shrink: 0;
  }
</style>
