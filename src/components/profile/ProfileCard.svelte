<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import Avatar from "$components/omni/Avatar.svelte";
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
  <Avatar tint={profile.skin?.tint} size={64} />
  <div class="profile-meta">
    <span class="profile-nick">{profile.nickname}</span>
    <span class="profile-hint">{$t("profile.fingerprint_label")}</span>
    <div class="fp-row">
      <code class="fp">{grouped}</code>
      <button
        type="button"
        class="button fp-copy"
        onclick={copy}
        aria-label={$t("profile.copy_fingerprint") as string}
      >
        {copied ? $t("profile.copied") : $t("profile.copy")}
      </button>
    </div>
  </div>
</div>

<style>
  .profile-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    padding: var(--space-3) 0;
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
    color: var(--text-dim);
  }

  .fp-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    min-width: 0;
  }

  .fp {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    letter-spacing: 0.02em;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .fp-copy {
    flex-shrink: 0;
  }
</style>
