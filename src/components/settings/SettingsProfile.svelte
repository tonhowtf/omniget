<script lang="ts">
  /**
   * Settings → General → Local profile. Mounted inside the "profile" SettingGroup,
   * with no props. Loads the profile once when the panel is first shown; no interval,
   * no polling. A failed read renders an error with retry, never the empty state of a
   * successful read, and a refresh failure keeps the last profile that loaded.
   */
  import { t } from "$lib/i18n";
  import ProfileCard from "$components/profile/ProfileCard.svelte";
  import ProfileEditor from "$components/profile/ProfileEditor.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";
  import {
    getProfile,
    getProfileError,
    isProfileAvailable,
    isProfileLoading,
    loadProfile,
  } from "$lib/stores/profile-store.svelte";

  let profile = $derived(getProfile());
  let loading = $derived(isProfileLoading());
  let available = $derived(isProfileAvailable());
  let errorKey = $derived(getProfileError());

  type View =
    | { kind: "loading" }
    | { kind: "ready"; refreshError: string | null }
    | { kind: "unavailable" }
    | { kind: "error"; key: string };

  let view = $derived.by<View>(() => {
    if (profile) return { kind: "ready", refreshError: errorKey };
    if (!available) return { kind: "unavailable" };
    if (errorKey && !loading) return { kind: "error", key: errorKey };
    return { kind: "loading" };
  });

  $effect(() => {
    loadProfile();
  });
</script>

<div class="settings-profile" aria-busy={view.kind === "loading"}>
  {#if view.kind === "ready" && profile}
    <ProfileCard {profile} />
    {#if view.refreshError}
      <SettingsEmptyState error title={$t(view.refreshError)} onRetry={() => loadProfile(true)} />
    {/if}
    <ProfileEditor embedded />
  {:else if view.kind === "loading"}
    <div class="profile-skeleton" role="status">
      <span class="skeleton-line" aria-hidden="true"></span>
      <span class="skeleton-line short" aria-hidden="true"></span>
      <span class="visually-hidden">{$t("profile.loading")}</span>
    </div>
  {:else if view.kind === "unavailable"}
    <SettingsEmptyState icon="general" title={$t("profile.empty_title")} hint={$t("profile.err_unavailable")} />
  {:else if view.kind === "error"}
    <SettingsEmptyState error title={$t(view.key)} onRetry={() => loadProfile(true)} />
  {/if}
</div>

<style>
  .settings-profile { display: flex; flex-direction: column; gap: var(--space-3); min-inline-size: 0; }
  .profile-skeleton { display: flex; flex-direction: column; gap: var(--space-2); padding-block: var(--space-2); }
  .skeleton-line { display: block; block-size: 20px; inline-size: 60%; border-radius: 6px; background: var(--surface-hi); }
  .skeleton-line.short { inline-size: 35%; }
  .visually-hidden { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
</style>
