<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { setToolbar } from "$lib/stores/toolbar-store.svelte";
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
    if (plugins.length === 0) {
      switchTab("browse");
    }

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

  $effect(() => {
    return setToolbar({
      segments: [
        { id: "installed", label: $t("marketplace.installed") as string, count: plugins.length },
        { id: "browse", label: $t("marketplace.browse") as string },
      ],
      activeSegment: activeTab,
      onSegment: (id) => switchTab(id as "installed" | "browse"),
    });
  });
</script>

<div class="marketplace-page page">

  {#if activeTab === "installed"}
    {#if loadingInstalled}
      <div class="loading">
        <span class="spinner"></span>
      </div>
    {:else if plugins.length === 0}
      <div class="empty">
        <img class="empty-state-art" src="/emoji/electric_plug.png" alt="" width="72" height="72" draggable="false" />
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
        <img class="empty-state-art" src="/emoji/satellite_antenna.png" alt="" width="72" height="72" draggable="false" />
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
    gap: var(--space-3);
    max-width: 760px;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-8) 0;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .empty p {
    margin: 0;
  }

  .error-hint {
    font-size: var(--text-sm);
    max-width: 360px;
  }

  .retry-btn {
    margin-top: var(--space-2);
  }

  .installed-hint {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text-dim);
    max-width: 60ch;
  }

  .plugin-section-label {
    margin: var(--space-3) 0 0;
    padding: 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    color: var(--text-dim);
  }

  /* App Store list: one grouped surface, 64pt rows, icon tile, trailing action */
  .plugin-list {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .plugin-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-3);
    position: relative;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .plugin-card + .plugin-card::before {
    content: "";
    position: absolute;
    top: 0;
    left: calc(var(--space-3) + 40px + var(--space-3));
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }

  @media (hover: hover) {
    .plugin-card:hover {
      background: var(--fill-1);
    }
  }

  .plugin-card.skeleton {
    pointer-events: none;
  }

  .skeleton-line {
    display: block;
    height: 12px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    animation: skeleton-shimmer 1.4s var(--ease-in-out) infinite;
  }

  .skeleton-line-name { width: 140px; height: 14px; }
  .skeleton-line-meta { width: 90px; height: 10px; margin-top: 6px; }
  .skeleton-line-action { width: 64px; height: 24px; border-radius: var(--radius-full); }
  .skeleton-line-desc { width: 70%; }
  .skeleton-line-desc-2 { width: 45%; }

  @keyframes skeleton-shimmer {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-line {
      animation: none;
    }
  }

  .plugin-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .plugin-header::before {
    content: "";
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    border-radius: 10px;
    background: linear-gradient(160deg, color-mix(in srgb, var(--accent) 85%, white), var(--accent-lo));
    box-shadow: inset 0 0 0 var(--hairline) rgba(0, 0, 0, 0.12);
    background-image:
      url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='rgba(30,18,0,0.85)' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M9.2 3.4h5.6v2.3a2 2 0 0 1-2 2h-1.6a2 2 0 0 1-2-2V3.4z'/><path d='M6.4 6.8h11.2c.88 0 1.6.72 1.6 1.6v4.7h-2.15a1.85 1.85 0 1 0 0 3.7H19.2v2.55c0 .88-.72 1.6-1.6 1.6H6.4c-.88 0-1.6-.72-1.6-1.6V16.8h2.15a1.85 1.85 0 1 0 0-3.7H4.8V8.4c0-.88.72-1.6 1.6-1.6z'/></svg>"),
      linear-gradient(160deg, color-mix(in srgb, var(--accent) 85%, white), var(--accent-lo));
    background-repeat: no-repeat;
    background-position: center;
    background-size: 22px 22px, cover;
  }

  .plugin-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .plugin-name-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .plugin-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge-official,
  .badge-community {
    display: inline-flex;
    align-items: center;
    height: 16px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    white-space: nowrap;
  }

  .badge-official {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }

  .badge-community {
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .installed-badge {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-dim);
  }

  .plugin-meta {
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .meta-sep {
    opacity: 0.5;
    margin: 0 2px;
  }

  .plugin-desc {
    margin: 0 0 0 calc(40px + var(--space-3));
    font-size: var(--text-sm);
    line-height: var(--leading-sm);
    color: var(--text-muted);
  }

  .plugin-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }

  .sidebar-toggle-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .sidebar-toggle-label {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .update-btn {
    height: 24px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--cta);
    color: var(--on-cta);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .update-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .update-hint {
    margin-left: calc(40px + var(--space-3));
    font-size: var(--text-sm);
    color: var(--accent-hi);
  }

  .load-error {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-left: calc(40px + var(--space-3));
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .load-error.incompatible {
    background: color-mix(in srgb, var(--warning) 12%, transparent);
  }

  .load-error strong {
    font-weight: 600;
  }

  .load-error code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    word-break: break-all;
  }

  .uninstall-btn {
    height: 24px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .uninstall-btn:hover {
      background: color-mix(in srgb, var(--danger) 12%, transparent);
      color: var(--danger);
    }
  }

  .uninstall-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  /* App Store "GET" pill */
  .install-btn {
    height: 26px;
    min-width: 72px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-full);
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.01em;
  }

  .install-btn:disabled {
    opacity: 0.6;
  }

  .tag-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-left: calc(40px + var(--space-3));
  }

  .tag {
    height: 18px;
    font-size: var(--text-caption);
  }

  .cap-details {
    margin-left: calc(40px + var(--space-3));
  }

  .cap-summary {
    font-size: var(--text-xs);
    color: var(--text-dim);
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
      color: var(--text);
    }
  }

  .cap-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: var(--space-1) 0 0;
    padding: 0;
    list-style: none;
  }

  .cap-item {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .cap-item svg {
    flex-shrink: 0;
    color: var(--text-dim);
  }

  @media (prefers-reduced-motion: reduce) {
    .loading :global(.spinner) {
      animation: mp-soft-pulse 1.5s var(--ease-in-out) infinite;
    }
  }

  @keyframes mp-soft-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }
</style>
