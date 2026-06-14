<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  type DependencyVariantInfo = {
    id: string;
    label: string;
    recommended: boolean;
  };

  type Props = {
    name: string;
    installed: boolean;
    version: string | null;
    busy: boolean;
    onInstall: (variant: string | null) => void | Promise<void>;
    onAfterCustomFile?: () => void | Promise<void>;
  };

  let { name, installed, version, busy, onInstall, onAfterCustomFile }: Props =
    $props();

  let variants = $state<DependencyVariantInfo[]>([]);
  let selectedVariant = $state<string | null>(null);
  let installDir = $state<string | null>(null);
  let menuOpen = $state(false);
  let loadingVariants = $state(false);

  async function loadVariants() {
    if (loadingVariants) return;
    loadingVariants = true;
    try {
      const list = await invoke<DependencyVariantInfo[]>("dependency_variants", {
        name,
      });
      variants = list;
      const recommended = list.find((v) => v.recommended);
      if (recommended && selectedVariant === null) {
        selectedVariant = recommended.id;
      } else if (list.length > 0 && selectedVariant === null) {
        selectedVariant = list[0].id;
      }
    } catch (e) {
      console.error("dependency_variants failed", e);
    } finally {
      loadingVariants = false;
    }
  }

  async function loadInstallDir() {
    try {
      installDir = await invoke<string>("dependency_install_dir", { name });
    } catch (e) {
      console.error("dependency_install_dir failed", e);
    }
  }

  $effect(() => {
    if (name) {
      void loadVariants();
      void loadInstallDir();
    }
  });

  async function handleInstall() {
    await onInstall(selectedVariant);
  }

  async function handlePickCustom() {
    menuOpen = false;
    const filters: { name: string; extensions: string[] }[] = [];
    if (name === "PDFium") {
      filters.push({ name: $t("settings.dependencies.filter_pdfium") as string, extensions: ["dll", "dylib", "so"] });
    }
    filters.push({ name: $t("settings.dependencies.filter_all_files") as string, extensions: ["*"] });
    try {
      const selected = await openDialog({
        title: $t("settings.dependencies.pick_dialog_title", { name }) as string,
        multiple: false,
        directory: false,
        filters,
      });
      if (!selected || typeof selected !== "string") return;
      const result = await invoke<string>("set_dependency_path", {
        name,
        sourcePath: selected,
      });
      showToast("success", `${name}: ${result}`);
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

  let supportsCustomFile = $derived(name === "PDFium");
</script>

<tr class="deps-table-row">
  <td class="deps-cell-name">
    <span class="deps-name">{name}</span>
  </td>
  <td class="deps-cell-version">
    {#if installed && version}
      <span class="deps-version">v{version}</span>
    {:else}
      <span class="deps-version deps-version-missing">—</span>
    {/if}
  </td>
  <td class="deps-cell-status">
    {#if installed}
      <span class="deps-status deps-status-ok">{$t("settings.dependencies.status_installed")}</span>
    {:else}
      <span class="deps-status deps-status-missing">{$t("settings.dependencies.status_missing")}</span>
    {/if}
  </td>
  <td class="deps-cell-action">
    <div class="deps-actions">
      {#if variants.length > 0}
        <select
          class="variant-select"
          value={selectedVariant ?? ""}
          onchange={(e) => (selectedVariant = (e.currentTarget as HTMLSelectElement).value)}
          disabled={busy}
          aria-label={$t("settings.dependencies.variant_aria") as string}
        >
          {#each variants as v (v.id)}
            <option value={v.id}>
              {v.label}{v.recommended ? " ★" : ""}
            </option>
          {/each}
        </select>
      {/if}

      {#if busy}
        <span class="dep-spinner" aria-hidden="true"></span>
      {:else}
        <button class="button dep-btn" onclick={handleInstall}>
          {#if installed}
            {$t("settings.dependencies.update")}
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
            {#if installDir}
              <div class="menu-path" title={installDir}>
                <code>{installDir}</code>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </td>
</tr>

<style>
  .deps-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .deps-version {
    font-size: 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--text-dim, var(--gray));
  }
  .deps-version-missing {
    opacity: 0.5;
  }
  .deps-status {
    display: inline-flex;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
  }
  .deps-status-ok {
    background: color-mix(in srgb, var(--success, #16a34a) 18%, transparent);
    color: var(--success, #16a34a);
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
  .variant-select {
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 6px);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    max-width: 160px;
  }
  .variant-select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
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
  .menu-btn {
    padding: 5px 10px;
    border: 1px solid var(--border);
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
    border: 1px solid var(--border);
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
    border-top: 1px solid var(--border);
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
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
