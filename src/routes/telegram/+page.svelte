<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type PluginStatus = "checking" | "ready" | "needs-restart" | "not-installed";
  let pluginStatus = $state<PluginStatus>("checking");

  onMount(async () => {
    try {
      const plugins = await invoke<{ id: string; enabled: boolean; loaded: boolean }[]>("list_plugins");
      const plugin = plugins.find((p) => p.id === "telegram");
      if (!plugin || !plugin.enabled) {
        pluginStatus = "not-installed";
      } else {
        pluginStatus = "ready";
      }
    } catch {
      pluginStatus = "ready";
    }
  });
</script>

{#if pluginStatus === "checking"}
  <div class="plugin-guard"><span class="spinner"></span></div>
{:else if pluginStatus === "not-installed"}
  <div class="plugin-guard">
    <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M21 5L2 12.5l7 1M21 5l-5.5 15-4.5-7.5M21 5L9 13.5" />
    </svg>
    <h2>{$t("marketplace.plugin_not_installed")}</h2>
    <p>{$t("marketplace.plugin_install_hint")}</p>
    <a href="/marketplace" class="guard-link">{$t("marketplace.go_to_marketplace")}</a>
  </div>
{:else if pluginStatus === "needs-restart"}
  <div class="plugin-guard">
    <h2>{$t("marketplace.restart_required")}</h2>
    <p>{$t("marketplace.plugin_restart_hint")}</p>
  </div>
{:else}
  <div class="plugin-guard">
    <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M21 5L2 12.5l7 1M21 5l-5.5 15-4.5-7.5M21 5L9 13.5" />
    </svg>
    <h2>Telegram</h2>
    <p>{$t("marketplace.restart_required")}</p>
  </div>
{/if}

<style>
  .plugin-guard { display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: calc(100vh - var(--padding) * 4); gap: calc(var(--padding) * 1.5); text-align: center; color: var(--gray); }
  .plugin-guard h2 { font-size: 18px; color: var(--secondary); }
  .plugin-guard p { font-size: 14px; max-width: 300px; }
  .guard-link { padding: 10px 24px; font-size: 14px; font-weight: 500; background: var(--cta); color: var(--on-cta); border-radius: var(--border-radius); text-decoration: none; }
  .spinner { width: 24px; height: 24px; border: 2px solid var(--input-border); border-top-color: var(--secondary); border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
