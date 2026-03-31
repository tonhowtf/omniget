<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type PluginNavInfo = {
    route: string;
    label: Record<string, string>;
    icon_svg: string | null;
    group: string;
    order: number;
  };

  type PluginInfo = {
    id: string;
    name: string;
    version: string;
    description: string;
    author: string;
    enabled: boolean;
    loaded: boolean;
    icon: string | null;
    nav: PluginNavInfo[];
  };

  type MarketplaceEntry = {
    id: string;
    name: string;
    description: string;
    author: string;
    repo: string;
    homepage: string | null;
    tags: string[];
    official: boolean;
    capabilities: string[];
    installed: boolean;
    installed_version: string | null;
  };

  const CAP_LABELS: Record<string, string> = {
    "core:events": "cap_events",
    "core:toast": "cap_toast",
    "core:settings": "cap_settings",
    "core:filesystem": "cap_filesystem",
    "core:proxy": "cap_proxy",
    "core:tools": "cap_tools",
    "core:download-queue": "cap_download_queue",
  };

  function capLabel(cap: string): string {
    const key = CAP_LABELS[cap];
    return key ? $t(`marketplace.${key}`) : cap;
  }

  type UpdateInfo = {
    id: string;
    installed_version: string;
    latest_version: string;
    repo: string;
    has_update: boolean;
  };

  let activeTab = $state<"installed" | "browse">("installed");
  let plugins = $state<PluginInfo[]>([]);
  let loadingInstalled = $state(true);
  let updates = $state<Record<string, UpdateInfo>>({});
  let updatingId = $state<string | null>(null);

  let registry = $state<MarketplaceEntry[]>([]);
  let loadingBrowse = $state(false);
  let browseError = $state(false);
  let browseFetched = $state(false);

  onMount(async () => {
    try {
      plugins = await invoke<PluginInfo[]>("list_plugins");
    } catch {
      plugins = [];
    }
    loadingInstalled = false;

    if (plugins.length > 0) {
      invoke<UpdateInfo[]>("check_plugin_updates")
        .then((updateList) => {
          for (const u of updateList) {
            if (u.has_update) updates[u.id] = u;
          }
        })
        .catch(() => {});
    }
  });

  async function loadBrowse() {
    if (browseFetched) return;
    loadingBrowse = true;
    browseError = false;
    try {
      const timeout = new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error("timeout")), 15000)
      );
      registry = await Promise.race([
        invoke<MarketplaceEntry[]>("fetch_marketplace_registry"),
        timeout,
      ]);
      browseFetched = true;
    } catch {
      browseError = true;
    }
    loadingBrowse = false;
  }

  function switchTab(tab: "installed" | "browse") {
    activeTab = tab;
    if (tab === "browse" && !browseFetched) {
      loadBrowse();
    }
  }

  async function uninstallPlugin(id: string) {
    try {
      await invoke("uninstall_plugin", { pluginId: id });
      plugins = plugins.filter((p) => p.id !== id);
      window.location.reload();
    } catch {}
  }

  let installingId = $state<string | null>(null);

  async function updatePlugin(id: string) {
    const info = updates[id];
    if (!info) return;
    updatingId = id;
    try {
      await invoke("update_plugin", { pluginId: id, repo: info.repo });
      delete updates[id];
      const idx = plugins.findIndex((p) => p.id === id);
      if (idx >= 0) {
        plugins[idx] = { ...plugins[idx], version: info.latest_version };
      }
    } catch {}
    updatingId = null;
  }

  async function installPlugin(id: string, repo: string) {
    installingId = id;
    try {
      await invoke("install_plugin_from_registry", { pluginId: id, repo });
      window.location.reload();
    } catch {}
    installingId = null;
  }

  async function togglePlugin(id: string, enabled: boolean) {
    try {
      await invoke("set_plugin_enabled", { pluginId: id, enabled });
      window.location.reload();
    } catch {}
  }
</script>

<div class="marketplace-page">
  <h1>{$t("marketplace.title")}</h1>

  <div class="tabs">
    <button
      class="tab"
      class:active={activeTab === "installed"}
      onclick={() => switchTab("installed")}
    >
      {$t("marketplace.installed")}
    </button>
    <button
      class="tab"
      class:active={activeTab === "browse"}
      onclick={() => switchTab("browse")}
    >
      {$t("marketplace.browse")}
    </button>
  </div>

  {#if activeTab === "installed"}
    {#if loadingInstalled}
      <div class="loading">
        <span class="spinner"></span>
      </div>
    {:else if plugins.length === 0}
      <div class="empty">
        <p>{$t("marketplace.no_plugins")}</p>
      </div>
    {:else}
      <div class="plugin-list">
        {#each plugins as plugin (plugin.id)}
          <div class="plugin-card">
            <div class="plugin-header">
              <div class="plugin-info">
                <span class="plugin-name">{plugin.name}</span>
                <span class="plugin-meta">
                  {$t("marketplace.version", { version: plugin.version })}
                  {#if plugin.author}
                    <span class="meta-sep">&middot;</span>
                    {$t("marketplace.by_author", { author: plugin.author })}
                  {/if}
                </span>
              </div>
              <div class="plugin-actions">
                {#if updates[plugin.id]}
                  <button
                    class="update-btn"
                    disabled={updatingId === plugin.id}
                    onclick={() => updatePlugin(plugin.id)}
                  >
                    {#if updatingId === plugin.id}
                      {$t("marketplace.browse_loading")}
                    {:else}
                      {$t("marketplace.update_available")}
                    {/if}
                  </button>
                {/if}
                <button
                  class="toggle-btn"
                  class:enabled={plugin.enabled}
                  onclick={() => togglePlugin(plugin.id, !plugin.enabled)}
                >
                  {plugin.enabled ? $t("marketplace.enabled") : $t("marketplace.disabled")}
                </button>
                <button
                  class="uninstall-btn"
                  onclick={() => uninstallPlugin(plugin.id)}
                >
                  {$t("marketplace.uninstall")}
                </button>
              </div>
            </div>
            {#if updates[plugin.id]}
              <span class="update-hint">{$t("marketplace.update_hint", { version: updates[plugin.id].latest_version })}</span>
            {/if}
            {#if plugin.description}
              <p class="plugin-desc">{plugin.description}</p>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

  {:else}
    {#if loadingBrowse}
      <div class="loading">
        <span class="spinner"></span>
        <span class="loading-text">{$t("marketplace.browse_loading")}</span>
      </div>
    {:else if browseError}
      <div class="empty">
        <p>{$t("marketplace.browse_error")}</p>
        <button class="retry-btn" onclick={loadBrowse}>{$t("marketplace.browse")}</button>
      </div>
    {:else if registry.length === 0}
      <div class="empty">
        <p>{$t("marketplace.browse_empty")}</p>
      </div>
    {:else}
      <div class="plugin-list">
        {#each registry as entry (entry.id)}
          <div class="plugin-card">
            <div class="plugin-header">
              <div class="plugin-info">
                <div class="plugin-name-row">
                  <span class="plugin-name">{entry.name}</span>
                  {#if entry.official}
                    <span class="badge-official">{$t("marketplace.official_badge")}</span>
                  {:else}
                    <span class="badge-community">{$t("marketplace.community_badge")}</span>
                  {/if}
                </div>
                <span class="plugin-meta">
                  {$t("marketplace.by_author", { author: entry.author })}
                </span>
              </div>
              <div class="plugin-actions">
                {#if entry.installed}
                  <span class="installed-badge">{$t("marketplace.installed_badge")}</span>
                {:else}
                  <button
                    class="install-btn"
                    disabled={installingId === entry.id}
                    onclick={() => installPlugin(entry.id, entry.repo)}
                  >
                    {#if installingId === entry.id}
                      {$t("marketplace.browse_loading")}
                    {:else}
                      {$t("marketplace.install")}
                    {/if}
                  </button>
                {/if}
              </div>
            </div>
            <p class="plugin-desc">{entry.description}</p>
            {#if entry.tags.length > 0}
              <div class="tag-list">
                {#each entry.tags as tag}
                  <span class="tag">{tag}</span>
                {/each}
              </div>
            {/if}
            {#if entry.capabilities.length > 0}
              <details class="cap-details">
                <summary class="cap-summary">{$t("marketplace.capabilities")} ({entry.capabilities.length})</summary>
                <ul class="cap-list">
                  {#each entry.capabilities as cap}
                    <li class="cap-item">
                      <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M9 12l2 2 4-4" />
                        <circle cx="12" cy="12" r="10" />
                      </svg>
                      {capLabel(cap)}
                    </li>
                  {/each}
                </ul>
              </details>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .marketplace-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 2);
    width: 100%;
    max-width: 700px;
    margin: 0 auto;
  }

  h1 {
    font-size: 20px;
    font-weight: 500;
  }

  .tabs {
    display: flex;
    gap: 2px;
    background: var(--button);
    border-radius: var(--border-radius);
    padding: 3px;
  }

  .tab {
    flex: 1;
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 500;
    color: var(--gray);
    background: none;
    border: none;
    border-radius: calc(var(--border-radius) - 3px);
    cursor: pointer;
  }

  .tab.active {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  @media (hover: hover) {
    .tab:not(.active):hover {
      color: var(--secondary);
    }
  }

  .loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
    padding: calc(var(--padding) * 4);
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--input-border);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .loading-text {
    font-size: 13px;
    color: var(--gray);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .empty {
    text-align: center;
    padding: calc(var(--padding) * 4);
    color: var(--gray);
    font-size: 14px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--padding);
  }

  .retry-btn {
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 500;
    background: var(--button);
    color: var(--secondary);
    border: none;
    border-radius: var(--border-radius);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .retry-btn:hover {
      background: var(--button-hover);
    }
  }

  .plugin-list {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .plugin-card {
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 1.5);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 0.75);
  }

  .plugin-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
  }

  .plugin-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .plugin-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .plugin-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .badge-official {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--cta);
    color: var(--on-cta);
  }

  .badge-community {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--button);
    color: var(--gray);
  }

  .installed-badge {
    font-size: 11px;
    font-weight: 500;
    color: var(--green);
  }

  .plugin-meta {
    font-size: 11.5px;
    color: var(--gray);
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .meta-sep {
    opacity: 0.5;
  }

  .plugin-desc {
    font-size: 13px;
    color: var(--gray);
    line-height: 1.4;
  }

  .plugin-actions {
    flex-shrink: 0;
  }

  .install-btn,
  .update-btn {
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    background: var(--cta);
    color: var(--on-cta);
  }

  .update-hint {
    font-size: 11px;
    color: var(--cta);
    font-weight: 500;
  }

  .uninstall-btn {
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    background: var(--button);
    color: var(--red);
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .uninstall-btn:hover {
      background: var(--button-hover);
    }
  }

  .install-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (hover: hover) {
    .install-btn:not(:disabled):hover {
      background: var(--cta-hover);
    }
  }

  .toggle-btn {
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    background: var(--button);
    color: var(--gray);
    box-shadow: var(--button-box-shadow);
  }

  .toggle-btn.enabled {
    background: var(--cta);
    color: var(--on-cta);
    box-shadow: none;
  }

  @media (hover: hover) {
    .toggle-btn:not(.enabled):hover {
      background: var(--button-hover);
      color: var(--secondary);
    }

    .toggle-btn.enabled:hover {
      background: var(--cta-hover);
    }
  }

  .toggle-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .tag-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .tag {
    font-size: 11px;
    font-weight: 500;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--button);
    color: var(--gray);
  }

  .cap-details {
    width: 100%;
  }

  .cap-summary {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    cursor: pointer;
    list-style: none;
    user-select: none;
  }

  .cap-summary::-webkit-details-marker {
    display: none;
  }

  .cap-summary::marker {
    content: "";
  }

  @media (hover: hover) {
    .cap-summary:hover {
      color: var(--secondary);
    }
  }

  .cap-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-top: 6px;
  }

  .cap-item {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--gray);
  }

  .cap-item svg {
    flex-shrink: 0;
    color: var(--green);
    pointer-events: none;
  }
</style>
