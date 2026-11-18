<script lang="ts">
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type Props = {
    name: string;
    installed: boolean;
    version: string | null;
    source?: string;
    path?: string | null;
    outdated?: boolean;
    busy: boolean;
    onInstall: () => void | Promise<void>;
    onAfterCustomFile?: () => void | Promise<void>;
  };

  let { name, installed, version, source = "missing", path = null, outdated = false, busy, onInstall, onAfterCustomFile }: Props =
    $props();

  let installDir = $state<string | null>(null);
  let menuOpen = $state(false);

  async function loadInstallDir() {
    try {
      installDir = await invoke<string>("dependency_install_dir", { name });
    } catch (e) {
      console.error("dependency_install_dir failed", e);
    }
  }

  // WHY: only name may retrigger this effect: anything else loops into ensure_ytdlp() and spawns yt-dlp and deno every pass (#281).
  $effect(() => {
    const dep = name;
    untrack(() => {
      if (!dep) return;
      void loadInstallDir();
      void loadArchived();
      void loadCustomPath();
    });
  });

  async function handlePickCustom() {
    menuOpen = false;
    try {
      const selected = await openDialog({
        title: $t("settings.dependencies.pick_dialog_title", { name }) as string,
        multiple: false,
        directory: false,
      });
      if (!selected || typeof selected !== "string") return;
      const result = await invoke<string>("set_dependency_path", {
        name,
        sourcePath: selected,
      });
      showToast("success", `${name}: ${result}`);
      void loadCustomPath();
      void onAfterCustomFile?.();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", $t("settings.dependencies.set_file_failed", { msg }) as string);
    }
  }

  async function handleShowFolder() {
    menuOpen = false;
    if (!installDir) return;
    try {
      await invoke("open_path_default", { path: installDir });
    } catch (e) {
      try {
        const shell = await import("@tauri-apps/plugin-shell");
        await shell.open(installDir);
      } catch (e2) {
        const msg = e2 instanceof Error ? e2.message : String(e2);
        showToast("error", $t("settings.dependencies.open_folder_failed", { msg }) as string);
      }
    }
  }

  function toggleMenu(e: Event) {
    e.stopPropagation();
    menuOpen = !menuOpen;
  }

  function onWindowClick() {
    if (menuOpen) menuOpen = false;
  }

  $effect(() => {
    if (menuOpen) {
      window.addEventListener("click", onWindowClick);
      return () => window.removeEventListener("click", onWindowClick);
    }
  });

  let supportsCustomFile = $derived(["yt-dlp", "FFmpeg"].includes(name));

  let customPath = $state<string | null>(null);
  let lastCustomPath = $state<string | null>(null);

  let sourceLabel = $derived(
    customPath ? ($t("settings.dependencies.custom_in_use") as string) : source === "system" ? "PATH" : source === "managed" ? ($t("settings.dependencies.source_managed") as string) : source === "flatpak" ? "Flatpak" : "",
  );

  async function loadCustomPath() {
    try {
      customPath = await invoke<string | null>("dependency_custom_path", { name });
      lastCustomPath = customPath ? null : await invoke<string | null>("dependency_last_custom_path", { name });
    } catch {
      customPath = null;
      lastCustomPath = null;
    }
  }

  async function handleUseLastCustom() {
    menuOpen = false;
    if (!lastCustomPath) return;
    try {
      const result = await invoke<string>("set_dependency_path", { name, sourcePath: lastCustomPath });
      showToast("success", `${name}: ${result}`);
      void loadCustomPath();
      void onAfterCustomFile?.();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", $t("settings.dependencies.set_file_failed", { msg }) as string);
    }
  }

  async function handleClearCustom() {
    menuOpen = false;
    try {
      await invoke("clear_dependency_path", { name });
      showToast("success", $t("settings.dependencies.custom_cleared") as string);
      void loadCustomPath();
      void onAfterCustomFile?.();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", $t("settings.dependencies.set_file_failed", { msg }) as string);
    }
  }

  type ArchivedVersion = { stamp_ms: string; path: string };
  let archived = $state<ArchivedVersion[]>([]);
  let rollingBack = $state<string | null>(null);

  async function loadArchived() {
    try {
      archived = await invoke<ArchivedVersion[]>("dependency_archived_versions", { name });
    } catch {
      archived = [];
    }
  }

  async function handleRollback(stampMs: string) {
    if (rollingBack) return;
    rollingBack = stampMs;
    try {
      const version = await invoke<string>("rollback_dependency", { name, stampMs });
      showToast("success", $t("settings.dependencies.rollback_done", { version }) as string);
      menuOpen = false;
      void onAfterCustomFile?.();
      void loadArchived();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", $t("settings.dependencies.rollback_failed", { msg }) as string);
    } finally {
      rollingBack = null;
    }
  }
</script>

<tr class="deps-table-row">
  <td class="deps-cell-name">
    <span class="deps-name" id={`dep-${name}-name`} dir="ltr">{name}</span>
    <span class="deps-meta" dir="ltr">
      {#if installed && version}v{version}{:else}—{/if}{#if sourceLabel} · <span title={customPath ?? path ?? ""}>{sourceLabel}</span>{/if}
    </span>
  </td>
  <td class="deps-cell-status">
    {#if installed && outdated}
      <span class="deps-status deps-status-outdated" title={$t("settings.dependencies.outdated_hint") as string}>
        {$t("settings.dependencies.status_outdated")}
      </span>
    {:else if installed}
      <span class="deps-status deps-status-ok">{$t("settings.dependencies.status_installed")}</span>
    {:else}
      <span class="deps-status deps-status-missing">{$t("settings.dependencies.status_missing")}</span>
    {/if}
  </td>
  <td class="deps-cell-action">
    <div class="deps-actions">
      {#if busy}
        <span class="dep-spinner" aria-hidden="true"></span>
        <span class="dep-busy-text" role="status">{$t("settings.rebrand.state.loading")}</span>
      {:else}
        <button class="button dep-btn" aria-describedby={`dep-${name}-name`} onclick={onInstall}>
          {#if installed}
            {source === "system" ? $t("settings.dependencies.download_managed") : $t("settings.dependencies.update")}
          {:else}
            {$t("settings.dependencies.install")}
          {/if}
        </button>
      {/if}

      <div class="menu-wrap">
        <button
          type="button"
          class="menu-btn"
          onclick={toggleMenu}
          disabled={busy}
          aria-label={$t("settings.dependencies.more_options") as string}
          aria-describedby={`dep-${name}-name`}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
        >
          ⋯
        </button>
        {#if menuOpen}
          <div class="menu" role="menu">
            {#if supportsCustomFile}
              <button type="button" class="menu-item" onclick={handlePickCustom} role="menuitem">
                {$t("settings.dependencies.pick_custom_file")}
              </button>
            {/if}
            <button
              type="button"
              class="menu-item"
              onclick={handleShowFolder}
              disabled={!installDir}
              role="menuitem"
            >
              {$t("settings.dependencies.show_folder")}
            </button>
            {#if customPath}
              <button type="button" class="menu-item" onclick={handleClearCustom} title={customPath} role="menuitem">
                {$t("settings.dependencies.use_managed_again")}
              </button>
            {:else if supportsCustomFile && lastCustomPath}
              <button type="button" class="menu-item" onclick={handleUseLastCustom} title={lastCustomPath} role="menuitem">
                {$t("settings.dependencies.use_custom_again")}
              </button>
            {/if}
            {#if archived.length > 0}
              <div class="menu-sep"></div>
              <div class="menu-label">{$t("settings.dependencies.previous_versions")}</div>
              {#each archived as v (v.stamp_ms)}
                <button
                  type="button"
                  class="menu-item"
                  onclick={() => handleRollback(v.stamp_ms)}
                  disabled={rollingBack !== null}
                  role="menuitem"
                >
                  {$t("settings.dependencies.rollback_to", {
                    when: new Date(Number(v.stamp_ms)).toLocaleString(),
                  })}
                </button>
              {/each}
            {/if}
            {#if path}
              <div class="menu-path" title={path}>
                <code dir="ltr">{path}</code>
              </div>
            {:else if installDir}
              <div class="menu-path" title={installDir}>
                <code dir="ltr">{installDir}</code>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </td>
</tr>

<style>
  .dep-busy-text {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .deps-name {
    display: block;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }
  .deps-meta {
    display: block;
    font-size: var(--text-xs);
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--text-muted);
  }
  .deps-status {
    display: inline-flex;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
  }
  .deps-status-ok {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
  }
  .deps-status-outdated {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
    color: var(--danger);
  }
  .deps-status-missing {
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text-dim, var(--gray));
  }
  .deps-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    flex-wrap: wrap;
  }
  .button.dep-btn {
    padding: 5px 12px;
    border-radius: var(--radius-sm, 6px);
    border: none;
    background: var(--accent);
    color: var(--on-accent, white);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  @media (hover: hover) {
    .button.dep-btn:hover {
      filter: brightness(1.05);
    }
  }
  .menu-wrap {
    position: relative;
  }
  .menu-sep {
    height: 1px;
    margin: 4px 0;
    background: var(--border);
  }
  .menu-label {
    padding: 4px 10px;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted);
  }
  .menu-btn {
    padding: 5px 10px;
    border: none;
    border-radius: var(--radius-sm, 6px);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
  }
  @media (hover: hover) {
    .menu-btn:hover:not(:disabled) {
      background: color-mix(in oklab, var(--text) 8%, transparent);
    }
  }
  .menu-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 50;
    min-width: 240px;
    background: var(--surface);
    border: none;
    border-radius: var(--radius-md, 8px);
    box-shadow: var(--elev-2, 0 8px 24px rgba(0, 0, 0, 0.2));
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .menu-item {
    padding: 8px 12px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    text-align: left;
    border-radius: 4px;
    cursor: pointer;
  }
  @media (hover: hover) {
    .menu-item:hover:not(:disabled) {
      background: color-mix(in oklab, var(--accent) 10%, transparent);
    }
  }
  .menu-item:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .menu-path {
    padding: 6px 12px 4px;
    border-top: none;
    margin-top: 4px;
  }
  .menu-path code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 10px;
    color: var(--text-dim, var(--gray));
    word-break: break-all;
    display: block;
    line-height: 1.4;
  }
  .dep-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid color-mix(in oklab, var(--text) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 800ms linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .dep-spinner { animation-duration: 2400ms; }
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
