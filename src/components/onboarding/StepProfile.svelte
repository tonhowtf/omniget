<script lang="ts">
  import { t } from "$lib/i18n";
  import SkinPicker from "$components/profile/SkinPicker.svelte";
  import {
    DEFAULT_SKIN_ID,
    DEFAULT_TINT,
    NICKNAME_MAX,
    getProfile,
    isProfileAvailable,
    loadProfile,
    normalizeNickname,
    sameTint,
    setNickname,
    setSkin,
    tintToCss,
  } from "$lib/stores/profile-store.svelte";

  let profile = $derived(getProfile());
  let available = $derived(isProfileAvailable());

  let draft = $state<string | null>(null);
  let value = $derived(draft ?? profile?.nickname ?? "");
  let tint = $derived(
    (profile?.skin?.tint ?? DEFAULT_TINT) as [number, number, number],
  );
  let initial = $derived(value.trim().slice(0, 1).toUpperCase() || "?");

  $effect(() => {
    loadProfile();
  });

  async function commitNickname() {
    if (normalizeNickname(value) === null) return;
    if (value.trim() === (profile?.nickname ?? "")) return;
    await setNickname(value);
  }

  async function pickTint(next: [number, number, number]) {
    if (sameTint(tint, next)) return;
    await setSkin(profile?.skin?.id ?? DEFAULT_SKIN_ID, next);
  }
</script>

<div class="step-profile">
  <div class="avatar" style:background={tintToCss(tint)} aria-hidden="true">
    <span class="avatar-initial">{initial}</span>
  </div>
  <h2>{$t("onboarding.profile.title")}</h2>
  <p class="step-desc">{$t("onboarding.profile.desc")}</p>

  <input
    type="text"
    class="input-text nick-input"
    maxlength={NICKNAME_MAX}
    value={value}
    placeholder={$t("onboarding.profile.placeholder") as string}
    aria-label={$t("onboarding.profile.nickname_label") as string}
    oninput={(e) => { draft = (e.target as HTMLInputElement).value; }}
    onblur={commitNickname}
    onkeydown={(e) => { if (e.key === "Enter") commitNickname(); }}
  />

  <SkinPicker {tint} onpick={pickTint} />

  <p class="step-note">
    {available ? $t("onboarding.profile.note") : $t("onboarding.profile.unavailable")}
  </p>
</div>

<style>
  .step-profile {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-2);
    width: 100%;
    margin: auto 0;
  }

  .avatar {
    width: 72px;
    height: 72px;
    border-radius: var(--radius-full);
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow:
      inset 0 0 0 var(--hairline) rgba(0, 0, 0, 0.18),
      var(--elev-1);
  }

  .avatar-initial {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    color: #fff;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
  }

  h2 {
    margin: var(--space-2) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .step-desc {
    font-size: var(--text-base);
    color: var(--text-muted);
    line-height: var(--leading-base);
    max-width: 340px;
    margin: 0;
  }

  .nick-input {
    width: 100%;
    max-width: 280px;
    margin-top: var(--space-3);
    text-align: center;
  }

  .step-note {
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: var(--leading-sm);
    max-width: 340px;
    margin: var(--space-2) 0 0;
  }
</style>
