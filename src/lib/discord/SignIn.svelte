<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import DcAvatar from "./DcAvatar.svelte";
  import TosNotice from "./TosNotice.svelte";
  import * as dc from "./api";
  import { dcState } from "./client.svelte";
  import { avatarUrl } from "./format";

  let { onSignedIn, onCancel }: { onSignedIn: () => void; onCancel?: () => void } = $props();

  type Method = "qr" | "token";
  let method = $state<Method>("qr");
  let token = $state("");
  let busy = $state(false);
  let tokenError = $state<string | null>(null);
  let startError = $state<string | null>(null);

  let qr = $derived(dcState.qr);
  let scannedUser = $derived(qr?.stage === "scanned" ? qr.user : null);

  async function startQr() {
    startError = null;
    dcState.qr = null;
    try {
      await dc.qrStart();
    } catch (e) {
      startError = dc.errKey(e);
    }
  }

  function setMethod(m: Method) {
    method = m;
    if (m === "qr") void startQr();
    else void dc.qrCancel().catch(() => {});
  }

  async function submitToken() {
    busy = true;
    tokenError = null;
    try {
      await dc.loginToken(token);
      token = "";
      onSignedIn();
    } catch (e) {
      tokenError = dc.errKey(e);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (qr?.stage === "done") onSignedIn();
  });

  onMount(() => {
    void startQr();
    return () => {
      void dc.qrCancel().catch(() => {});
      dcState.qr = null;
    };
  });
</script>

<section class="signin">
  <header>
    <h2>{$t("discord.signin.title")}</h2>
    <p>{$t("discord.signin.subtitle")}</p>
  </header>

  <TosNotice />

  <div class="segmented segmented-lg methods" role="tablist">
    <button class="segmented-btn" role="tab" aria-selected={method === "qr"} onclick={() => setMethod("qr")}>{$t("discord.signin.method_qr")}</button>
    <button class="segmented-btn" role="tab" aria-selected={method === "token"} onclick={() => setMethod("token")}>{$t("discord.signin.method_token")}</button>
  </div>

  {#if method === "qr"}
    <div class="card surface-card qr-card">
      <div class="qr-box" class:dim={qr?.stage !== "code"} aria-label={$t("discord.signin.qr_alt")}>
        {#if qr?.stage === "code"}
          {@html qr.svg}
        {:else if scannedUser}
          <DcAvatar src={avatarUrl({ ...scannedUser, global_name: null }, 128)} name={scannedUser.username} size={72} />
        {:else if qr?.stage === "expired" || qr?.stage === "cancelled" || qr?.stage === "error" || startError}
          <TgIcon name="warn" size={28} />
        {:else}
          <span class="spinner"></span>
        {/if}
      </div>
      <div class="qr-copy" aria-live="polite">
        {#if scannedUser}
          <h3>{$t("discord.signin.scanned_title", { name: scannedUser.username })}</h3>
          <p class="muted">{$t("discord.signin.scanned_hint")}</p>
          <button class="btn btn-ghost btn-sm" onclick={() => void startQr()}>{$t("discord.signin.not_you")}</button>
        {:else if qr?.stage === "done"}
          <h3>{$t("discord.signin.signing_in")}</h3>
        {:else if qr?.stage === "expired" || qr?.stage === "cancelled"}
          <h3>{$t(qr.stage === "expired" ? "discord.signin.expired" : "discord.signin.cancelled")}</h3>
          <button class="btn btn-secondary btn-sm" onclick={() => void startQr()}><TgIcon name="refresh" /> {$t("discord.signin.new_code")}</button>
        {:else if qr?.stage === "error" || startError}
          <h3>{$t("discord.signin.qr_failed")}</h3>
          <p class="field-error">{$t(`discord.errors.${qr?.stage === "error" ? dc.errKey(qr.error) : startError}`)}</p>
          <button class="btn btn-secondary btn-sm" onclick={() => void startQr()}><TgIcon name="refresh" /> {$t("discord.common.retry")}</button>
        {:else}
          <h3>{$t("discord.signin.qr_title")}</h3>
          <ol>
            <li>{$t("discord.signin.qr_step1")}</li>
            <li>{$t("discord.signin.qr_step2")}</li>
            <li>{$t("discord.signin.qr_step3")}</li>
          </ol>
        {/if}
      </div>
    </div>
  {:else}
    <form class="card surface-card stack" onsubmit={(e) => { e.preventDefault(); void submitToken(); }}>
      <label class="field">
        <span class="field-label">{$t("discord.signin.token_label")}</span>
        <input class="input input-lg mono" type="password" bind:value={token} autocomplete="off" spellcheck="false" placeholder="MTA…" />
        <span class="field-hint">{$t("discord.signin.token_where")}</span>
      </label>
      <p class="keep"><TgIcon name="shield" /> {$t("discord.signin.token_storage")}</p>
      {#if tokenError}<p class="field-error">{$t(`discord.errors.${tokenError}`)}</p>{/if}
      <div class="btn-row end">
        <button type="submit" class="btn btn-primary" disabled={busy || token.trim().length < 50}>
          {#if busy}<span class="spinner"></span>{/if}{$t("discord.signin.sign_in")}
        </button>
      </div>
    </form>
  {/if}

  {#if onCancel}
    <button class="btn btn-ghost back" onclick={onCancel}>{$t("discord.common.back")}</button>
  {/if}
</section>

<style>
  .signin {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 560px;
    width: 100%;
    margin-inline: auto;
  }

  header h2 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  header p,
  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .methods {
    align-self: flex-start;
  }

  .card {
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .card h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .qr-card {
    flex-direction: row;
    align-items: center;
    gap: var(--space-5);
  }

  .qr-box {
    width: 188px;
    height: 188px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    padding: 10px;
    border-radius: var(--radius-lg);
    background: #fff;
    color: #8a8a8e;
  }

  .qr-box.dim {
    background: var(--fill-1);
    color: var(--text-dim);
  }

  .qr-box :global(svg) {
    width: 100%;
    height: 100%;
  }

  .qr-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
    min-width: 0;
  }

  .qr-copy ol {
    margin: 0;
    padding-left: 1.2em;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.7;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .keep {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.5;
  }

  .back {
    align-self: center;
  }

  @media (max-width: 560px) {
    .qr-card {
      flex-direction: column;
    }
  }
</style>
