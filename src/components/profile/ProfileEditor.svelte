<script lang="ts">
  /**
   * Nickname field + skin picker. One primary action (Save) for the nickname;
   * the tint saves on click, the way a macOS colour well does.
   */
  import { t } from "$lib/i18n";
  import SkinPicker from "./SkinPicker.svelte";
  import {
    DEFAULT_SKIN_ID,
    DEFAULT_TINT,
    NICKNAME_MAX,
    getProfile,
    getProfileError,
    normalizeNickname,
    sameTint,
    setNickname,
    setSkin,
  } from "$lib/stores/profile-store.svelte";

  // `embedded`: inside a Settings group (SettingsProfile), which already draws
  // the section, so the editor drops its own card.
  let { onsaved, embedded = false }: { onsaved?: () => void; embedded?: boolean } = $props();
  const uid = $props.id();
  const nickId = `profile-nickname-${uid}`;
  const nickHintId = `${nickId}-hint`;

  let profile = $derived(getProfile());
  let draft = $state<string | null>(null);
  let saving = $state(false);

  let value = $derived(draft ?? profile?.nickname ?? "");
  let normalized = $derived(normalizeNickname(value));
  let tint = $derived(
    (profile?.skin?.tint ?? DEFAULT_TINT) as [number, number, number],
  );
  let dirty = $derived(normalized !== null && normalized !== (profile?.nickname ?? ""));
  let errorKey = $derived(getProfileError());

  async function save() {
    if (!dirty || saving) return;
    saving = true;
    const ok = await setNickname(value);
    saving = false;
    if (ok) {
      draft = null;
      onsaved?.();
    }
  }

  async function pickTint(next: [number, number, number]) {
    if (sameTint(tint, next) || saving) return;
    saving = true;
    await setSkin(profile?.skin?.id ?? DEFAULT_SKIN_ID, next);
    saving = false;
  }
</script>

<div class:card={!embedded} class="profile-editor" data-setting-key="profile.nickname">
  <div class="setting-row">
    <div class="setting-col">
      <label class="setting-label" for={nickId}>{$t("profile.nickname_label")}</label>
      <span class="setting-path" id={nickHintId}><bdi>{$t("profile.nickname_desc")}</bdi></span>
    </div>
    <div class="nick-controls">
      <input
        type="text"
        id={nickId}
        aria-describedby={nickHintId}
        class="input-text nick-input"
        maxlength={NICKNAME_MAX}
        value={value}
        placeholder={$t("profile.nickname_placeholder") as string}
        aria-invalid={value.length > 0 && normalized === null}
        oninput={(e) => { draft = (e.target as HTMLInputElement).value; }}
        onkeydown={(e) => { if (e.key === "Enter") save(); }}
      />
      <button class="button" onclick={save} disabled={!dirty || saving}>
        {$t("profile.save")}
      </button>
    </div>
  </div>
  <div class="divider"></div>
  <div class="setting-row skin-row">
    <div class="setting-col">
      <span class="setting-label">{$t("profile.skin_label")}</span>
      <span class="setting-path"><bdi>{$t("profile.skin_desc")}</bdi></span>
    </div>
    <SkinPicker {tint} disabled={saving} onpick={pickTint} />
  </div>
  {#if errorKey}
    <div class="divider"></div>
    <div class="setting-row">
      <span class="profile-error" role="status">{$t(errorKey)}</span>
    </div>
  {/if}
</div>

<style>
  .nick-controls {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex: 0 1 auto;
    min-width: 0;
  }

  .nick-input {
    width: 220px;
    max-width: 100%;
  }

  .profile-editor .setting-row {
    flex-wrap: wrap;
  }

  /* Help text wraps instead of ending in an ellipsis (the global .setting-path clips). */
  .profile-editor .setting-col {
    flex: 1 1 14rem;
  }

  .profile-editor .setting-path {
    white-space: normal;
    overflow: visible;
    text-overflow: clip;
    overflow-wrap: anywhere;
  }

  .skin-row {
    flex-wrap: wrap;
  }

  .profile-error {
    font-size: var(--text-sm);
    color: var(--danger);
  }
</style>
