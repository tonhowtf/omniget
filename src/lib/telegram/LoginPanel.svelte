<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import TgIcon from "./TgIcon.svelte";
  import { tg, errText, type QrStart } from "./api";

  let { onAuthenticated, onTrouble }: { onAuthenticated: (phone: string) => void; onTrouble: () => void } = $props();

  type Method = "qr" | "phone";
  let method = $state<Method>("qr");
  let needPassword = $state(false);

  let qr = $state<QrStart | null>(null);
  let qrError = $state<string | null>(null);
  let qrTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;

  let phone = $state("");
  let code = $state("");
  let codeSent = $state(false);
  let phoneBusy = $state(false);
  let phoneError = $state<string | null>(null);

  let password = $state("");
  let passwordHint = $state("");
  let passwordBusy = $state(false);
  let passwordError = $state<string | null>(null);

  function stopQr() {
    if (qrTimer) clearTimeout(qrTimer);
    qrTimer = null;
  }

  async function startQr() {
    stopQr();
    qrError = null;
    qr = null;
    try {
      qr = await tg.qrStart();
      schedulePoll();
    } catch (e) {
      qrError = errText(e);
    }
  }

  function schedulePoll() {
    stopQr();
    if (disposed || method !== "qr" || needPassword) return;
    qrTimer = setTimeout(pollQr, 1500);
  }

  async function pollQr() {
    if (qr && qr.expires && Date.now() / 1000 > qr.expires - 1) {
      await startQr();
      return;
    }
    try {
      const s = await tg.qrPoll();
      if (s === "waiting") return schedulePoll();
      if (s.startsWith("success")) return onAuthenticated(s.split(":").slice(1).join(":"));
      if (s.startsWith("password_required")) return askPassword(s);
      qrError = s;
    } catch (e) {
      qrError = errText(e);
      schedulePoll();
    }
  }

  function askPassword(s: string) {
    const i = s.indexOf(":");
    passwordHint = i >= 0 ? s.slice(i + 1) : "";
    needPassword = true;
    stopQr();
  }

  async function sendCode() {
    phoneBusy = true;
    phoneError = null;
    try {
      await tg.sendCode(phone.replace(/[\s()-]/g, ""));
      codeSent = true;
    } catch (e) {
      phoneError = errText(e);
    } finally {
      phoneBusy = false;
    }
  }

  async function verifyCode() {
    phoneBusy = true;
    phoneError = null;
    try {
      const r = await tg.verifyCode(code.trim());
      if (r.startsWith("password_required")) return askPassword(r);
      onAuthenticated(r);
    } catch (e) {
      const msg = errText(e);
      if (msg.startsWith("password_required")) return askPassword(msg);
      phoneError = msg === "invalid_code" ? ($t("telegram.login.invalid_code") as string) : msg;
    } finally {
      phoneBusy = false;
    }
  }

  async function verifyPassword() {
    passwordBusy = true;
    passwordError = null;
    try {
      const r = await tg.verify2fa(password);
      onAuthenticated(typeof r === "string" ? r : "");
    } catch (e) {
      passwordError = errText(e);
    } finally {
      passwordBusy = false;
    }
  }

  function setMethod(m: Method) {
    method = m;
    if (m === "qr") void startQr();
    else stopQr();
  }

  onMount(() => {
    void startQr();
  });

  onDestroy(() => {
    disposed = true;
    stopQr();
  });
</script>

<section class="login">
  <img class="hero" src="/telegram/hero.webp" alt="" width="996" height="689" decoding="async" draggable="false" />
  <header>
    <h2>{$t("telegram.login.title")}</h2>
    <p>{$t("telegram.login.subtitle")}</p>
  </header>

  {#if needPassword}
    <div class="card surface-card">
      <h3>{$t("telegram.login.password_title")}</h3>
      <p class="muted">{passwordHint ? $t("telegram.login.password_hint", { hint: passwordHint }) : $t("telegram.login.password_text")}</p>
      <form class="stack" onsubmit={(e) => { e.preventDefault(); void verifyPassword(); }}>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input input-lg" type="password" bind:value={password} autocomplete="current-password" autofocus aria-label={$t("telegram.login.password_title")} />
        {#if passwordError}<p class="field-error">{passwordError}</p>{/if}
        <div class="btn-row end">
          <button type="button" class="btn btn-ghost" onclick={() => { needPassword = false; setMethod(method); }}>{$t("telegram.common.back")}</button>
          <button type="submit" class="btn btn-primary" disabled={passwordBusy || !password}>
            {#if passwordBusy}<span class="spinner"></span>{/if}{$t("telegram.login.sign_in")}
          </button>
        </div>
      </form>
    </div>
  {:else}
    <div class="segmented segmented-lg methods" role="tablist">
      <button class="segmented-btn" role="tab" aria-selected={method === "qr"} onclick={() => setMethod("qr")}>{$t("telegram.login.method_qr")}</button>
      <button class="segmented-btn" role="tab" aria-selected={method === "phone"} onclick={() => setMethod("phone")}>{$t("telegram.login.method_phone")}</button>
    </div>

    {#if method === "qr"}
      <div class="card surface-card qr-card">
        <div class="qr-box" aria-label={$t("telegram.login.qr_alt")}>
          {#if qr}
            {@html qr.svg}
          {:else if qrError}
            <TgIcon name="warn" size={28} />
          {:else}
            <span class="spinner"></span>
          {/if}
        </div>
        <div class="qr-copy">
          <h3>{$t("telegram.login.qr_title")}</h3>
          <ol>
            <li>{$t("telegram.login.qr_step1")}</li>
            <li>{$t("telegram.login.qr_step2")}</li>
            <li>{$t("telegram.login.qr_step3")}</li>
          </ol>
          {#if qrError}
            <p class="field-error">{qrError}</p>
            <button class="btn btn-secondary btn-sm" onclick={() => void startQr()}><TgIcon name="refresh" /> {$t("telegram.common.retry")}</button>
          {/if}
        </div>
      </div>
    {:else}
      <div class="card surface-card">
        {#if !codeSent}
          <form class="stack" onsubmit={(e) => { e.preventDefault(); void sendCode(); }}>
            <label class="field">
              <span class="field-label">{$t("telegram.login.phone_label")}</span>
              <input class="input input-lg" type="tel" bind:value={phone} placeholder="+86 138 0000 0000" autocomplete="tel" />
              <span class="field-hint">{$t("telegram.login.phone_hint")}</span>
            </label>
            {#if phoneError}<p class="field-error">{phoneError}</p>{/if}
            <div class="btn-row end">
              <button type="submit" class="btn btn-primary" disabled={phoneBusy || phone.replace(/\D/g, "").length < 6}>
                {#if phoneBusy}<span class="spinner"></span>{/if}{$t("telegram.login.send_code")}
              </button>
            </div>
          </form>
        {:else}
          <form class="stack" onsubmit={(e) => { e.preventDefault(); void verifyCode(); }}>
            <label class="field">
              <span class="field-label">{$t("telegram.login.code_label")}</span>
              <!-- svelte-ignore a11y_autofocus -->
              <input class="input input-lg code" type="text" inputmode="numeric" bind:value={code} autocomplete="one-time-code" autofocus />
              <span class="field-hint">{$t("telegram.login.code_hint", { phone })}</span>
            </label>
            {#if phoneError}<p class="field-error">{phoneError}</p>{/if}
            <div class="btn-row end">
              <button type="button" class="btn btn-ghost" onclick={() => { codeSent = false; code = ""; }}>{$t("telegram.login.change_number")}</button>
              <button type="submit" class="btn btn-primary" disabled={phoneBusy || code.trim().length < 4}>
                {#if phoneBusy}<span class="spinner"></span>{/if}{$t("telegram.login.sign_in")}
              </button>
            </div>
          </form>
        {/if}
      </div>
    {/if}
  {/if}

  <button class="trouble" onclick={onTrouble}>
    <TgIcon name="plug" /> {$t("telegram.login.trouble")}
  </button>
</section>

<style>
  .login {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 560px;
    width: 100%;
    margin-inline: auto;
  }

  .hero {
    display: block;
    align-self: center;
    width: clamp(220px, 42vw, 440px);
    height: auto;
    margin-bottom: calc(var(--space-2) * -1);
    user-select: none;
    pointer-events: none;
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

  .code {
    font-family: var(--font-mono);
    letter-spacing: 0.3em;
    font-size: var(--text-lg);
  }

  .trouble {
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: var(--space-1) var(--space-2);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--accent-hi);
    cursor: pointer;
  }

  .trouble:hover {
    text-decoration: underline;
  }

  @media (max-width: 560px) {
    .qr-card {
      flex-direction: column;
    }
  }
</style>
