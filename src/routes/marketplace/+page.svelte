<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type PluginNavInfo = {
    route: string;
    label: Record<string, string>;
    icon_svg: string | null;
    group: string;
    order: number;
  };

  type PluginLoadError = {
    message: string;
    kind: string;
    plugin_abi?: number | null;
    expected_abi?: number | null;
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
    load_error?: PluginLoadError | null;
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

  async function refreshPlugins() {
    try {
      plugins = await invoke<PluginInfo[]>("list_plugins");
    } catch {
      plugins = [];
    }
  }

  onMount(async () => {
    await refreshPlugins();
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
      delete updates[id];
      const idx = registry.findIndex((p) => p.id === id);
      if (idx >= 0) {
        registry[idx] = { ...registry[idx], installed: false, installed_version: null };
      }
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e?.message ?? $t("common.error");
      showToast("error", msg);
    }
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
      await refreshPlugins();
      const idx = registry.findIndex((p) => p.id === id);
      if (idx >= 0) {
        const installed = plugins.find((p) => p.id === id);
        registry[idx] = {
          ...registry[idx],
          installed: true,
          installed_version: installed?.version ?? registry[idx].installed_version,
        };
      }
    } catch (e: any) {
      const raw = typeof e === "string" ? e : e?.message ?? $t("common.error");
      const msg = raw.startsWith("NetworkUnreachable|")
        ? $t("marketplace.install_network_error")
        : raw;
      showToast("error", msg);
    }
    installingId = null;
  }

  async function togglePlugin(id: string, enabled: boolean) {
    try {
      await invoke("set_plugin_enabled", { pluginId: id, enabled });
      const idx = plugins.findIndex((p) => p.id === id);
      if (idx >= 0) plugins[idx] = { ...plugins[idx], enabled };
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e?.message ?? $t("common.error");
      showToast("error", msg);
    }
  }

  let sidebarPlugins = $derived(plugins.filter((p) => p.enabled));
  let hiddenPlugins = $derived(plugins.filter((p) => !p.enabled));
</script>

<div class="marketplace-page">

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
        <button class="btn btn-primary retry-btn" onclick={() => switchTab("browse")}>
          {$t("marketplace.browse")}
        </button>
      </div>
    {:else}
      <p class="installed-hint">{$t("marketplace.installed_hint")}</p>
      {#if sidebarPlugins.length > 0}
        <h5 class="plugin-section-label">{$t("marketplace.section_in_sidebar")}</h5>
        <div class="plugin-list">
          {#each sidebarPlugins as plugin (plugin.id)}
            {@render installedCard(plugin)}
          {/each}
        </div>
      {/if}
      {#if hiddenPlugins.length > 0}
        <h5 class="plugin-section-label">{$t("marketplace.section_hidden")}</h5>
        <div class="plugin-list">
          {#each hiddenPlugins as plugin (plugin.id)}
            {@render installedCard(plugin)}
          {/each}
        </div>
      {/if}
    {/if}

  {:else}
    {#if loadingBrowse}
      <div class="plugin-list">
        {#each [0, 1, 2] as i (i)}
          <div class="plugin-card skeleton">
            <div class="plugin-header">
              <div class="plugin-info">
                <span class="skeleton-line skeleton-line-name"></span>
                <span class="skeleton-line skeleton-line-meta"></span>
              </div>
              <span class="skeleton-line skeleton-line-action"></span>
            </div>
            <span class="skeleton-line skeleton-line-desc"></span>
            <span class="skeleton-line skeleton-line-desc-2"></span>
          </div>
        {/each}
      </div>
    {:else if browseError}
      <div class="empty">
        <p>{$t("marketplace.browse_error")}</p>
        <p class="error-hint">{$t("marketplace.browse_error_hint")}</p>
        <button class="btn btn-secondary retry-btn" onclick={loadBrowse}>{$t("marketplace.browse_retry")}</button>
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
                    class="btn btn-primary install-btn"
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

{#snippet installedCard(plugin: PluginInfo)}
  <div class="plugin-card" class:active-sidebar={plugin.enabled}>
    <div class="plugin-header">
      <div class="plugin-info">
        <div class="plugin-name-row">
          <span class="plugin-name">{plugin.name}</span>
          <span class="status-pill" class:on={plugin.enabled}>
            {plugin.enabled ? $t("marketplace.status_in_sidebar") : $t("marketplace.status_hidden")}
          </span>
        </div>
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
        <div class="sidebar-toggle-row">
          <span class="sidebar-toggle-label">{$t("marketplace.show_in_sidebar")}</span>
          <button
            class="toggle"
            class:on={plugin.enabled}
            role="switch"
            aria-checked={plugin.enabled}
            aria-label={$t("marketplace.show_in_sidebar")}
            onclick={() => togglePlugin(plugin.id, !plugin.enabled)}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
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
    {#if plugin.enabled && !plugin.loaded && plugin.load_error}
      {@const incompatible =
        plugin.load_error.kind === "abi_mismatch" ||
        plugin.load_error.kind === "missing_abi_symbol"}
      <div class="load-error" class:incompatible>
        <strong>
          {incompatible
            ? $t("marketplace.plugin_incompatible_title")
            : $t("marketplace.plugin_load_failed_title")}
        </strong>
        <span>
          {incompatible
            ? $t("marketplace.plugin_incompatible_hint")
            : $t("marketplace.plugin_load_failed_hint")}
        </span>
        <code>{plugin.load_error.message}</code>
      </div>
    {/if}
    {#if plugin.description}
      <p class="plugin-desc">{plugin.description}</p>
    {/if}
  </div>
{/snippet}

<style>
  .marketplace-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 2);
    width: 100%;
    max-width: 720px;
    align-items: stretch;
  }

  h1 {
    font-size: 20px;
    font-weight: 500;
  }

  .tabs {
    display: inline-flex;
    align-self: flex-start;
    gap: 2px;
    background: var(--button);
    border-radius: var(--border-radius);
    padding: 3px;
    width: fit-content;
    max-width: 100%;
  }

  .tab {
    flex: 0 1 auto;
    min-width: 7rem;
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

  .error-hint {
    max-width: 420px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--gray);
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
    background: var(--surface);
    border-radius: var(--radius-md);
    box-shadow: var(--elev-1);
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    transition: transform var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
  }

  .plugin-card.skeleton {
    pointer-events: none;
  }

  .skeleton-line {
    display: block;
    height: 12px;
    border-radius: var(--radius-xs);
    background: linear-gradient(
      90deg,
      var(--surface-mut) 0%,
      var(--surface-hi) 50%,
      var(--surface-mut) 100%
    );
    background-size: 200% 100%;
    animation: skeleton-shimmer 1.5s ease-in-out infinite;
  }

  .skeleton-line-name { width: 40%; height: 16px; margin-bottom: var(--space-1); }
  .skeleton-line-meta { width: 25%; height: 11px; }
  .skeleton-line-action { width: 80px; height: 28px; border-radius: var(--radius-sm); }
  .skeleton-line-desc { width: 92%; height: 11px; margin-top: var(--space-2); }
  .skeleton-line-desc-2 { width: 60%; height: 11px; }

  @keyframes skeleton-shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-line {
      animation: none;
      background: var(--surface-mut);
    }
  }

  @media (hover: hover) {
    .plugin-card:hover {
      transform: translateY(-1px);
      box-shadow: var(--elev-2);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .plugin-card {
      transition: none;
    }
    .plugin-card:hover {
      transform: none;
    }
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
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .installed-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.5;
    margin: 0;
  }

  .plugin-section-label {
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
    margin: 0 0 var(--space-2);
  }

  .plugin-card.active-sidebar {
    background: color-mix(in srgb, var(--accent) 6%, var(--surface));
  }

  .status-pill {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: var(--radius-full);
    background: var(--surface-hi);
    color: var(--text-muted);
  }

  .status-pill.on {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }

  .sidebar-toggle-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .sidebar-toggle-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .toggle {
    position: relative;
    width: 44px;
    height: 24px;
    border-radius: var(--radius-full);
    background: var(--surface-hi);
    border: none;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: background var(--duration-base) var(--ease-out);
  }

  .toggle.on {
    background: var(--accent);
  }

  .toggle-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full);
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
    transition: transform var(--duration-base) var(--ease-out);
    pointer-events: none;
  }

  .toggle.on .toggle-knob {
    transform: translateX(20px);
  }

  .toggle:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (prefers-reduced-motion: reduce) {
    .toggle, .toggle-knob {
      transition: none;
    }
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

  .load-error {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: var(--border-radius);
    background: color-mix(in srgb, var(--error, #c1121f) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--error, #c1121f) 35%, transparent);
    font-size: 12px;
    color: var(--secondary);
  }

  .load-error.incompatible {
    background: color-mix(in srgb, var(--warning, #f59e0b) 12%, transparent);
    border-color: color-mix(in srgb, var(--warning, #f59e0b) 35%, transparent);
  }

  .load-error strong {
    font-size: 12px;
    color: var(--secondary);
  }

  .load-error code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    word-break: break-word;
    background: var(--surface);
    padding: 2px 6px;
    border-radius: 4px;
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
