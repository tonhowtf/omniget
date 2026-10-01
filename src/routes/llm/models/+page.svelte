<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import RoutingRules from "$components/llm/RoutingRules.svelte";
  import PruneSettings from "$components/llm/PruneSettings.svelte";
  import WatchdogSettings from "$components/llm/WatchdogSettings.svelte";

  type Kind = { id: string; name: string; base_url: string };
  type KeyView = { id: string; name: string; kind: string; has_key: boolean; models: number };
  type PkceStart = { url: string; state: string; callback_url: string };
  type AuthResult = { ok: true; key: KeyView } | { ok: false; error: string };

  let kinds = $state<Kind[]>([]);
  let keys = $state<KeyView[]>([]);
  let loading = $state(true);
  let loadFailed = $state(false);
  let signingIn = $state(false);
  let authReady = $state(false);
  let authLoadFailed = $state(false);
  let authUrl = $state("");
  let authExpired = $state(false);
  let reopening = $state(false);
  let alive = true;
  let loadGeneration = 0;
  let authGeneration = 0;
  let authTimer: ReturnType<typeof setTimeout> | null = null;
  let unlistenAuth: (() => void) | null = null;

  async function loadKeys() {
    const generation = ++loadGeneration;
    loading = true;
    loadFailed = false;
    const results = await Promise.allSettled([
      invoke<Kind[]>("tool_keys_kinds"),
      invoke<KeyView[]>("tool_keys_list"),
    ]);
    if (!alive || generation !== loadGeneration) return;
    const [providerResult, keyResult] = results;
    if (providerResult.status === "fulfilled" && Array.isArray(providerResult.value)) kinds = providerResult.value;
    else loadFailed = true;
    if (keyResult.status === "fulfilled" && Array.isArray(keyResult.value)) keys = keyResult.value;
    else loadFailed = true;
    loading = false;
  }

  function stopWaiting() {
    authGeneration++;
    if (authTimer) clearTimeout(authTimer);
    authTimer = null;
    signingIn = false;
    authUrl = "";
  }

  onMount(() => {
    void loadKeys();
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const stop = await listen<AuthResult>("llm://openrouter-auth", (message) => {
          if (!alive) return;
          stopWaiting();
          authExpired = false;
          const result = message.payload;
          if (result?.ok) {
            void loadKeys();
            showToast("success", $t("llm.models.openrouter_ok"));
          } else showToast("error", $t("llm.models.openrouter_failed", { error: result?.error ?? "" }));
        });
        if (alive) { unlistenAuth = stop; authReady = true; }
        else stop();
      } catch {
        unlistenAuth = null;
        authLoadFailed = true;
      }
    })();
  });

  onDestroy(() => {
    alive = false;
    loadGeneration++;
    stopWaiting();
    unlistenAuth?.();
    unlistenAuth = null;
  });

  function kindName(id: string): string {
    return kinds.find((k) => k.id === id)?.name ?? id;
  }

  async function reopenSignIn() {
    if (!authUrl || reopening) return;
    reopening = true;
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(authUrl);
    } catch { showToast("error", $t("llm.models.openrouter_unavailable")); }
    finally { reopening = false; }
  }

  async function signInOpenRouter() {
    if (signingIn) return;
    if (!unlistenAuth) {
      showToast("error", $t("llm.models.openrouter_unavailable"));
      return;
    }
    const generation = ++authGeneration;
    signingIn = true;
    authExpired = false;
    authTimer = setTimeout(() => {
      if (!alive || generation !== authGeneration) return;
      stopWaiting();
      authExpired = true;
    }, 120_000);
    try {
      const start = await invoke<PkceStart>("tool_ai_keys_openrouter_pkce");
      if (!alive || generation !== authGeneration) return;
      if (!start?.url) throw new Error("ERR_PKCE_NO_START");
      authUrl = start.url;
      await reopenSignIn();
    } catch {
      if (!alive || generation !== authGeneration) return;
      stopWaiting();
      showToast("error", $t("llm.models.openrouter_unavailable"));
    }
  }
</script>

<svelte:head><title>{$t("llm.models.title")}</title></svelte:head>

<div class="page page-wide models-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.models.title")}</h1>
    </div>
    <button type="button" class="button primary" disabled={signingIn || !authReady} onclick={signInOpenRouter}>
      {$t("llm.models.sign_in_openrouter")}
    </button>
  </header>
  {#if authLoadFailed}<p role="alert">{$t("llm.models.openrouter_unavailable")}</p>{/if}
  {#if signingIn}
    <div class="auth-status" role="status">
      <p>{$t("llm.models.openrouter_started")}</p>
      <p>{$t("llm.models.stop_waiting_hint")}</p>
      <div class="auth-actions">
        <button type="button" class="button" disabled={!authUrl || reopening} onclick={reopenSignIn}>{$t("llm.models.reopen_browser")}</button>
        <button type="button" class="button" onclick={stopWaiting}>{$t("llm.models.stop_waiting")}</button>
      </div>
    </div>
  {:else if authExpired}
    <p role="status">{$t("llm.models.auth_expired")}</p>
  {/if}
  <SurfaceGuide text={$t("llm.surface.models_hint")} href="/help?article=models#guide" />

  <div class="group">
    <div class="group-label">{$t("llm.models.providers")}</div>
    {#if loading}<p role="status">{$t("llm.models.loading")}</p>{/if}
    {#if loadFailed}
      <div class="provider-error" role="alert">
        <p>{$t("llm.models.providers_failed")}</p>
        <button type="button" class="button" disabled={loading} onclick={loadKeys}>{$t("llm.surface.retry")}</button>
      </div>
    {/if}
    {#if !loading && !loadFailed && keys.length === 0}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.models.no_providers")}</div>
          <div class="group-row-sub">{$t("llm.models.add_key_hint")}</div>
        </div>
      </div>
    {:else}
      {#each keys as key (key.id)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{key.name || kindName(key.kind)}</div>
            <div class="group-row-sub">{kindName(key.kind)}</div>
          </div>
          <div class="group-row-trailing">
            {#if key.models > 0}
              <span class="tag">{$t("llm.models.count", { count: key.models })}</span>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <a class="button" href="/llm/accounts">{$t("llm.models.connect")} →</a>
  <details class="routing-details"><summary>{$t("llm.surface.advanced")}</summary>
  <RoutingRules />
  <PruneSettings />
  <WatchdogSettings />
  </details>
</div>

<style>
  .auth-status, .provider-error { padding: var(--space-4); margin-bottom: var(--space-4); background: var(--fill-1); border-radius: var(--radius-md); }
  .auth-status p, .provider-error p { margin: 0 0 var(--space-3); font-size: var(--text-sm); }
  .auth-actions { display: flex; gap: var(--space-2); flex-wrap: wrap; }
 .routing-details { margin-top:24px; } .routing-details summary { padding:16px 0; font-weight:600; cursor:pointer; }
  .models-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .models-page :global(.group) {
    margin-bottom: var(--space-5);
  }
</style>
