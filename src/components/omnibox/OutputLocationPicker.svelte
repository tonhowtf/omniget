<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";

  let {
    selectedOutputDir = $bindable<string>(""),
  } = $props();

  let settings = $derived(getSettings());
  let savedDirs = $derived(settings?.download.saved_output_dirs ?? []);
  let defaultDir = $derived(settings?.download.default_output_dir ?? "");

  let isDropdownOpen = $state(false);
  let mutating = $state(false);
  let pendingWrites = 0;
  let writeChain: Promise<void> = Promise.resolve();

  // If no output dir is selected yet, default to the default_output_dir
  $effect(() => {
    if (!selectedOutputDir && defaultDir) {
      selectedOutputDir = defaultDir;
    }
  });

  function enqueueSavedDirsUpdate(mutator: (dirs: string[]) => string[]) {
    pendingWrites += 1;
    mutating = true;
    const run = writeChain
      .then(async () => {
        const current = getSettings()?.download.saved_output_dirs ?? [];
        const next = mutator(current);
        if (next.length === current.length && next.every((dir, i) => dir === current[i])) {
          return;
        }
        await updateSettings({ download: { saved_output_dirs: next } });
      })
      .finally(() => {
        pendingWrites -= 1;
        if (pendingWrites === 0) mutating = false;
      });
    writeChain = run.catch(() => {});
    return run;
  }

  async function handleBrowse() {
    isDropdownOpen = false;
    const picked = await open({
      directory: true,
      title: $t("settings.download.choose_folder"),
    });
    if (!picked) return;

    const newPath = picked as string;
    selectedOutputDir = newPath;

    if (newPath === defaultDir) return;
    await enqueueSavedDirsUpdate((dirs) =>
      dirs.includes(newPath) ? dirs : [...dirs, newPath],
    );
  }

  function pickDir(path: string) {
    selectedOutputDir = path;
    isDropdownOpen = false;
  }

  async function removeDir(e: MouseEvent, path: string) {
    e.stopPropagation();
    if (selectedOutputDir === path) {
      selectedOutputDir = defaultDir;
    }
    await enqueueSavedDirsUpdate((dirs) => dirs.filter((d) => d !== path));
  }
</script>

<div class="location-picker">
  <button
    type="button"
    class="location-picker-trigger"
    onclick={() => { isDropdownOpen = !isDropdownOpen; }}
    aria-haspopup="listbox"
    aria-expanded={isDropdownOpen}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
    </svg>
    <span class="location-picker-label">{$t('settings.download.section_output')}:</span>
    <span class="location-picker-value" title={selectedOutputDir}>{selectedOutputDir || defaultDir}</span>
    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="6 9 12 15 18 9" />
    </svg>
  </button>

  {#if isDropdownOpen}
    <ul class="location-picker-menu" role="listbox">
      <!-- Default Directory -->
      <li class="location-picker-item-wrapper">
        <button
          type="button"
          class="location-picker-option"
          class:active={selectedOutputDir === defaultDir}
          role="option"
          aria-selected={selectedOutputDir === defaultDir}
          onclick={() => pickDir(defaultDir)}
        >
          <span class="location-picker-option-alias">{$t('settings.download.default_output_dir')}</span>
          <span class="location-picker-option-meta" title={defaultDir}>{defaultDir}</span>
        </button>
      </li>

      <!-- Saved Directories -->
      {#each savedDirs as dir}
        <li class="location-picker-item-wrapper">
          <button
            type="button"
            class="location-picker-option"
            class:active={selectedOutputDir === dir}
            role="option"
            aria-selected={selectedOutputDir === dir}
            onclick={() => pickDir(dir)}
          >
            <span class="location-picker-option-meta" title={dir}>{dir}</span>
          </button>
          <button 
            type="button" 
            class="location-picker-remove" 
            onclick={(e) => removeDir(e, dir)}
            disabled={mutating}
            aria-label="Remove"
            title="Remove"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </li>
      {/each}

      <!-- Browse Option -->
      <li class="location-picker-item-wrapper">
        <button
          type="button"
          class="location-picker-option browse-btn"
          role="option"
          aria-selected="false"
          disabled={mutating}
          onclick={handleBrowse}
        >
          <span class="location-picker-option-alias">{$t('settings.download.choose_folder')}...</span>
        </button>
      </li>
    </ul>
  {/if}
</div>

<style>
  .location-picker {
    position: relative;
    display: inline-flex;
    width: 100%;
    max-width: 100%;
  }

  .location-picker-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--gray);
    background: var(--button);
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
    width: 100%;
    text-align: left;
    overflow: hidden;
  }

  .location-picker-trigger:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (hover: hover) {
    .location-picker-trigger:hover {
      background: var(--button-hover);
      color: var(--secondary);
    }
  }

  .location-picker-label {
    color: var(--gray);
    white-space: nowrap;
  }

  .location-picker-value {
    color: var(--secondary);
    font-weight: 600;
    flex-grow: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .location-picker-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 30;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--popup-bg);
    border-radius: var(--border-radius);
    box-shadow: var(--elev-2);
    max-height: 250px;
    overflow-y: auto;
  }

  .location-picker-item-wrapper {
    display: flex;
    align-items: stretch;
    width: 100%;
  }

  .location-picker-option {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 2px;
    flex-grow: 1;
    padding: 8px 10px;
    font-size: var(--text-sm);
    color: var(--secondary);
    background: transparent;
    border: none;
    border-radius: calc(var(--border-radius) - 4px);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
  }

  .location-picker-remove {
    background: transparent;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 0 10px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .location-picker-remove:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .location-picker-remove:hover {
    color: var(--danger);
  }

  @media (hover: hover) {
    .location-picker-option:hover {
      background: var(--button-hover);
    }
  }

  .location-picker-option.active {
    background: var(--button-elevated);
  }

  .location-picker-option-alias {
    color: var(--secondary);
    font-weight: 500;
  }

  .location-picker-option-meta {
    color: var(--gray);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    width: 100%;
    direction: rtl;
    text-align: left;
  }

  .browse-btn {
    align-items: center;
    flex-direction: row;
    color: var(--cta);
  }
  
  .browse-btn .location-picker-option-alias {
    color: var(--cta);
  }
</style>
