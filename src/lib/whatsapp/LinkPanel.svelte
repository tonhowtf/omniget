<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import { wa } from "./session.svelte";
  import WaIcon from "./WaIcon.svelte";

  type Mode = "qr" | "phone";

  let mode = $state<Mode>("qr");
  let phone = $state("");
  let busy = $state(false);
  let localError = $state<string | null>(null);

  let status = $derived(wa.status);
  let running = $derived(status?.state === "starting" || status?.state === "pairing");
  let code = $derived(status?.pair_code ?? null);
  let shownError = $derived(localError ?? status?.error ?? null);

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? $t(`whatsapp.errors.${k}`) : raw;
  }

  function prettyCode(c: string): string {
    return c.length === 8 ? `${c.slice(0, 4)}-${c.slice(4)}` : c;
  }

  async function start() {
    busy = true;
    localError = null;
    try {
      await api.connect();
    } catch (e) {
      localError = api.errText(e);
    } finally {
      busy = false;
    }
  }

  async function stop() {
    busy = true;
    try {
      await api.disconnect();
    } catch (e) {
      localError = api.errText(e);
    } finally {
      busy = false;
    }
  }

  async function requestCode(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    localError = null;
    try {
      await api.pairPhone(phone);
    } catch (e) {
      localError = api.errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="link card">
  <div class="link-copy">
    <h2>{$t("whatsapp.link.title")}</h2>
    <p class="link-hint">{$t("whatsapp.link.hint")}</p>

    <div class="mac-segmented link-modes" role="tablist" aria-label={$t("whatsapp.link.title")}>
      <button type="button" role="tab" class="mac-segmented-btn" class:active={mode === "qr"} aria-selected={mode === "qr"} onclick={() => (mode = "qr")}>
        <WaIcon name="qr" /> {$t("whatsapp.link.mode_qr")}
      </button>
      <button type="button" role="tab" class="mac-segmented-btn" class:active={mode === "phone"} aria-selected={mode === "phone"} onclick={() => (mode = "phone")}>
        <WaIcon name="phone" /> {$t("whatsapp.link.mode_phone")}
      </button>
    </div>

    {#if mode === "qr"}
      <ol class="steps">
        <li>{$t("whatsapp.link.step_open")}</li>
        <li>{$t("whatsapp.link.step_devices")}</li>
        <li>{$t("whatsapp.link.step_scan")}</li>
      </ol>
    {:else}
      <form class="phone-form" onsubmit={requestCode}>
        <label class="field">
          <span class="field-label">{$t("whatsapp.link.phone_label")}</span>
          <input class="input" type="tel" inputmode="tel" autocomplete="tel" bind:value={phone} placeholder="+55 11 91234-5678" disabled={busy} />
          <span class="field-hint">{$t("whatsapp.link.phone_hint")}</span>
        </label>
        <button class="btn btn-primary" type="submit" disabled={busy || phone.replace(/\D/g, "").length < 8}>
          {#if busy}<span class="spinner"></span>{/if}
          {$t("whatsapp.link.get_code")}
        </button>
      </form>
      {#if code}
        <div class="pair-code" aria-live="polite">
          <span class="pair-code-label">{$t("whatsapp.link.code_label")}</span>
          <span class="pair-code-value">{prettyCode(code)}</span>
          <span class="field-hint">{$t("whatsapp.link.code_hint")}</span>
        </div>
      {/if}
    {/if}

    {#if shownError}
      <div class="notice notice-warning" role="alert">
        <div class="notice-body">
          <div class="notice-text">{errorLabel(shownError)}</div>
        </div>
      </div>
    {/if}

    <p class="privacy">{$t("whatsapp.link.privacy")}</p>
  </div>

  <div class="qr-box">
    {#if mode === "qr" && status?.qr_svg}
      <div class="qr" role="img" aria-label={$t("whatsapp.link.qr_alt")}>
        {@html status.qr_svg}
      </div>
      <button class="btn btn-ghost btn-sm" onclick={stop} disabled={busy}>{$t("whatsapp.link.cancel")}</button>
    {:else if running}
      <div class="qr qr-wait">
        <span class="spinner"></span>
        <span>{$t("whatsapp.link.preparing")}</span>
      </div>
      <button class="btn btn-ghost btn-sm" onclick={stop} disabled={busy}>{$t("whatsapp.link.cancel")}</button>
    {:else}
      <div class="qr qr-wait">
        <WaIcon name="qr" size={40} />
      </div>
      <button class="btn btn-primary" onclick={start} disabled={busy}>
        {#if busy}<span class="spinner"></span>{/if}
        {status?.error === "qr_expired" ? $t("whatsapp.link.new_qr") : $t("whatsapp.link.start")}
      </button>
    {/if}
  </div>
</section>

<style>
  .link {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--space-6);
    align-items: start;
    padding: var(--space-5);
  }

  .link-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .link h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .link-hint,
  .privacy {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--text-dim);
  }

  .privacy {
    color: var(--text-muted);
  }

  .link-modes {
    align-self: flex-start;
  }

  .steps {
    margin: 0;
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-base);
    color: var(--text);
    line-height: 1.5;
  }

  .phone-form {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    max-width: 360px;
  }

  .phone-form .field {
    width: 100%;
  }

  .pair-code {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    align-self: flex-start;
  }

  .pair-code-label {
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }

  .pair-code-value {
    font-family: var(--font-mono);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--text);
    user-select: all;
  }

  .qr-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .qr {
    width: 248px;
    height: 248px;
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .qr :global(svg) {
    width: 100%;
    height: 100%;
  }

  .qr-wait {
    flex-direction: column;
    gap: var(--space-2);
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  @media (max-width: 720px) {
    .link {
      grid-template-columns: 1fr;
    }
  }
</style>
