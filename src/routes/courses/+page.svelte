<script lang="ts">
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { COURSE_PLATFORMS, type CoursePlatform } from "$lib/courses/platforms";
  import { PLATFORM_ICONS, DEFAULT_ICON } from "./platform-icons";
  import { t } from "$lib/i18n";

  type PluginStatus = "checking" | "ready" | "needs-restart" | "not-installed";
  let pluginStatus = $state<PluginStatus>("checking");

  let searchQuery = $state("");
  let authStatus: Record<string, { checked: boolean; email: string | null; error: boolean }> = $state({});

  let enabledPlatforms = $derived(
    COURSE_PLATFORMS.filter((p) => p.enabled)
  );

  let disabledPlatforms = $derived(
    COURSE_PLATFORMS.filter((p) => !p.enabled)
  );

  let filteredEnabled = $derived(
    searchQuery.trim() === ""
      ? enabledPlatforms
      : enabledPlatforms.filter((p) =>
          p.name.toLowerCase().includes(searchQuery.trim().toLowerCase())
        )
  );

  let filteredDisabled = $derived(
    searchQuery.trim() === ""
      ? disabledPlatforms
      : disabledPlatforms.filter((p) =>
          p.name.toLowerCase().includes(searchQuery.trim().toLowerCase())
        )
  );

  onMount(async () => {
    try {
      const plugins = await invoke<{ id: string; enabled: boolean; loaded: boolean }[]>("list_plugins");
      const courses = plugins.find((p) => p.id === "courses");
      if (!courses || !courses.enabled) {
        pluginStatus = "not-installed";
        return;
      }
      pluginStatus = "ready";
    } catch {
      pluginStatus = "ready";
    }

    for (const platform of COURSE_PLATFORMS) {
      if (platform.enabled && platform.authCheckCommand) {
        authStatus[platform.id] = { checked: false, email: null, error: false };
        invoke<string>(platform.authCheckCommand)
          .then((email) => {
            authStatus[platform.id] = { checked: true, email, error: false };
          })
          .catch(() => {
            authStatus[platform.id] = { checked: true, email: null, error: true };
          });
      }
    }
  });

  function handleCardClick(platform: CoursePlatform) {
    if (!platform.enabled) return;
    goto(platform.route);
  }

  function handleKeyDown(e: KeyboardEvent, platform: CoursePlatform) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      handleCardClick(platform);
    }
  }

  function getIconSvg(icon: string): string {
    return PLATFORM_ICONS[icon] ?? DEFAULT_ICON;
  }
</script>

{#if pluginStatus === "checking"}
  <div class="plugin-guard"><span class="spinner"></span></div>
{:else if pluginStatus === "not-installed"}
  <div class="plugin-guard">
    <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M4 19.5A2.5 2.5 0 016.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z" />
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
<div class="courses-page">
  <h1>{$t("courses.title")}</h1>

  <input
    class="search-input"
    type="text"
    placeholder={$t("courses.search_placeholder")}
    bind:value={searchQuery}
  />

  <div class="platform-grid">
    {#each filteredEnabled as platform (platform.id)}
      <div
        class="platform-card"
        role="button"
        tabindex={0}
        onclick={() => handleCardClick(platform)}
        onkeydown={(e) => handleKeyDown(e, platform)}
      >
        <div class="card-icon" style="--platform-color: {platform.color}">
          <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            {@html getIconSvg(platform.icon)}
          </svg>
        </div>
        <span class="card-name">{platform.name}</span>
        <span class="card-status">
          {#if authStatus[platform.id]?.checked && authStatus[platform.id]?.error}
            <span class="status-dot error"></span>
            {$t("courses.connection_failed")}
          {:else if authStatus[platform.id]?.checked && authStatus[platform.id]?.email}
            <span class="status-dot connected"></span>
            <span class="status-email">{authStatus[platform.id].email}</span>
          {:else if authStatus[platform.id]?.checked}
            <span class="status-dot disconnected"></span>
            {$t("courses.not_connected")}
          {/if}
        </span>
      </div>
    {/each}
  </div>

  {#if filteredDisabled.length > 0}
    <details class="coming-soon-section">
      <summary class="coming-soon-toggle">
        {$t("courses.coming_soon")} ({filteredDisabled.length})
      </summary>
      <div class="coming-soon-list">
        {#each filteredDisabled as platform (platform.id)}
          <div class="coming-soon-item">
            <div class="coming-soon-icon" style="--platform-color: {platform.color}">
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                {@html getIconSvg(platform.icon)}
              </svg>
            </div>
            <span>{platform.name}</span>
          </div>
        {/each}
      </div>
    </details>
  {/if}
</div>
{/if}

<style>
  .courses-page {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--padding) * 2);
    width: 100%;
  }

  h1 {
    font-size: 20px;
    font-weight: 500;
    margin-block: 0;
    width: 100%;
    max-width: 900px;
  }

  .search-input {
    width: 100%;
    max-width: 900px;
    padding: 10px var(--padding);
    font-size: 14px;
    color: var(--secondary);
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    outline: none;
    box-sizing: border-box;
  }

  .search-input:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .search-input::placeholder {
    color: var(--gray);
  }

  .platform-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--padding);
    width: 100%;
    max-width: 900px;
    justify-items: center;
  }

  .platform-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
    gap: calc(var(--padding) * 0.75);
    padding: calc(var(--padding) * 2) var(--padding);
    background: var(--button-elevated);
    border-radius: var(--border-radius);
    cursor: pointer;
    transition: transform 0.15s, background 0.15s;
  }

  @media (hover: hover) {
    .platform-card:hover {
      background: var(--sidebar-highlight);
      transform: translateY(-2px);
    }
  }

  .platform-card:active {
    transform: translateY(0);
  }

  .platform-card:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .card-icon {
    width: 52px;
    height: 52px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) - 2px);
    background: color-mix(in srgb, var(--platform-color) 15%, transparent);
    color: var(--platform-color);
  }

  .card-name {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .card-status {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    min-height: 16px;
  }

  .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .status-dot.connected {
    background: var(--green);
  }

  .status-dot.disconnected {
    background: var(--gray);
  }

  .status-dot.error {
    background: var(--red);
  }

  .status-email {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
  }

  .coming-soon-section {
    width: 100%;
    max-width: 900px;
  }

  .coming-soon-toggle {
    font-size: 13px;
    font-weight: 500;
    color: var(--gray);
    cursor: pointer;
    list-style: none;
    user-select: none;
  }

  .coming-soon-toggle::-webkit-details-marker {
    display: none;
  }

  .coming-soon-toggle::marker {
    content: "";
  }

  @media (hover: hover) {
    .coming-soon-toggle:hover {
      color: var(--secondary);
    }
  }

  .coming-soon-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: var(--padding);
  }

  .coming-soon-item {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) * 0.75);
    padding: 6px var(--padding);
    font-size: 13px;
    color: var(--gray);
    border-radius: calc(var(--border-radius) - 4px);
  }

  .coming-soon-icon {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    background: color-mix(in srgb, var(--platform-color) 10%, transparent);
    color: var(--platform-color);
    opacity: 0.5;
  }

  @media (max-width: 535px) {
    .platform-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .platform-card {
      transition: none;
    }
  }

  .plugin-guard {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
    gap: calc(var(--padding) * 1.5);
    text-align: center;
    color: var(--gray);
  }
  .plugin-guard h2 { font-size: 18px; color: var(--secondary); }
  .plugin-guard p { font-size: 14px; max-width: 300px; }
  .guard-link { padding: 10px 24px; font-size: 14px; font-weight: 500; background: var(--cta); color: var(--on-cta); border-radius: var(--border-radius); text-decoration: none; }
  .spinner { width: 24px; height: 24px; border: 2px solid var(--input-border); border-top-color: var(--secondary); border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
