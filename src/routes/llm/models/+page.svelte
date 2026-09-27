<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  /**
   * Models & Routing: the providers the user has keys for, the routing rule and
   * "Sign in with OpenRouter".
   *
   * The PKCE flow is split: `tool_ai_keys_openrouter_pkce` (f2-secrets) hands
   * back `{ url, state, callback_url }` without touching the network, this page
   * opens `url` in the browser, and the `omniget://openrouter-auth` deep link is
   * finished in `lib.rs`, which emits `llm://openrouter-auth` with
   * `{ ok: true, key }` or `{ ok: false, error }`. The listener lives only while
   * this page is mounted. Nothing here calls the network on mount.
   */
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
  let signingIn = $state(false);
  let unlistenAuth: (() => void) | null = null;

  async function loadKeys() {
    try {
      keys = (await invoke<KeyView[] | null>("tool_keys_list")) ?? keys;
    } catch {
      // Keeps whatever the page already showed.
    }
  }

  onMount(async () => {
    try {
      kinds = (await invoke<Kind[] | null>("tool_keys_kinds")) ?? [];
    } catch {
      kinds = [];
    }
    try {
      keys = (await invoke<KeyView[] | null>("tool_keys_list")) ?? [];
    } catch {
      keys = [];
    }
    try {
      const { listen } = await import("@tauri-apps/api/event");
      unlistenAuth = await listen<AuthResult>("llm://openrouter-auth", (message) => {
        signingIn = false;
        const result = message.payload;
        if (result?.ok) {
          void loadKeys();
          showToast("success", $t("llm.models.openrouter_ok"));
        } else {
          showToast("error", $t("llm.models.openrouter_failed", { error: result?.error ?? "" }));
        }
      });
    } catch {
      // No event bridge (browser): the button still reports failures inline.
    }
  });

  onDestroy(() => {
    unlistenAuth?.();
    unlistenAuth = null;
  });

  function kindName(id: string): string {
    return kinds.find((k) => k.id === id)?.name ?? id;
  }

  async function signInOpenRouter() {
    signingIn = true;
    try {
      const start = await invoke<PkceStart>("tool_ai_keys_openrouter_pkce");
      if (!start?.url) throw new Error("ERR_PKCE_NO_START");
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(start.url);
      // `signingIn` stays true until `llm://openrouter-auth` answers: the user is
      // in the browser and there is nothing to do here in the meantime.
      showToast("info", $t("llm.models.openrouter_started"));
    } catch {
      signingIn = false;
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
    <button type="button" class="button primary" disabled={signingIn} onclick={signInOpenRouter}>
      {$t("llm.models.sign_in_openrouter")}
    </button>
  </header>
  <SurfaceGuide text={$t("llm.surface.models_hint")} href="/help?article=models#guide" />

  <div class="group">
    <div class="group-label">{$t("llm.models.providers")}</div>
    {#if keys.length === 0}
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
              <span class="tag">{$t("llm.models.count", { n: key.models })}</span>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <a class="button" href="/settings?tab=ai">{$t("llm.models.connect")} →</a>
  <details class="routing-details"><summary>{$t("llm.surface.advanced")}</summary>
  <RoutingRules />
  </details>
  <PruneSettings />
  <WatchdogSettings />
</div>

<style>
 .routing-details { margin-top:24px; } .routing-details summary { padding:16px 0; font-weight:600; cursor:pointer; }
  /* Same reason as the roster page: flex children would shrink and clip. */
  .models-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .models-page :global(.group) {
    margin-bottom: var(--space-5);
  }
</style>
