<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import type { DependencyStatus } from "../settings-helpers";
  import DependencyRow from "../DependencyRow.svelte";
  import SettingsEmptyState from "../rebrand/SettingsEmptyState.svelte";

  type PortableInfo = { is_portable: boolean; data_dir: string | null; macos_webview_notice: boolean };

  let deps = $state<DependencyStatus[]>([]);
  let loading = $state(false);
  let loaded = $state(false);
  let failed = $state(false);
  let installing = $state<string | null>(null);
  let rowErrors = $state<Record<string, string>>({});
  let portable = $state<PortableInfo | null>(null);

  async function loadDeps() {
    if (loading) return;
    loading = true;
    failed = false;
    try { deps = await invoke<DependencyStatus[]>("check_dependencies"); loaded = true; }
    catch { failed = true; }
    finally { loading = false; }
  }

  async function loadPortable() {
    try { portable = await invoke<PortableInfo>("get_portable_info"); }
    catch { portable = null; }
  }

  async function install(name: string) {
    if (installing) return;
    const wasInstalled = deps.find((d) => d.name === name)?.installed ?? false;
    installing = name;
    if (rowErrors[name]) {
      const next = { ...rowErrors };
      delete next[name];
      rowErrors = next;
    }
    try {
      const version = await invoke<string>("install_dependency", { name, force: wasInstalled });
      await loadDeps();
      await refreshYtdlpStatus();
      showToast("success", $t(wasInstalled ? "settings.dependencies.update_success" : "settings.dependencies.install_success", { name, version }) as string);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const brew = name === "FFmpeg" ? "ffmpeg" : name === "yt-dlp" ? "yt-dlp" : null;
      const key = wasInstalled ? "settings.dependencies.update_failed" : brew ? "settings.dependencies.install_failed" : "settings.dependencies.install_failed_retry";
      const formatted = $t(key, { name, msg, brew: brew ?? "" }) as string;
      rowErrors = { ...rowErrors, [name]: formatted };
      showToast("error", formatted);
      await loadDeps();
    } finally {
      installing = null;
    }
  }

  onMount(() => {
    void loadDeps();
    void loadPortable();
  });
</script>

{#if portable?.is_portable}
  <div class="portable" role="status" data-setting-key="settings.storage.portable_label">
    <p class="portable-label">{$t("settings.storage.portable_label")}</p>
    <p class="muted"><bdi>{$t("settings.storage.portable_active", { dir: portable.data_dir ?? "<app>/data" })}</bdi></p>
    {#if portable.macos_webview_notice}<p class="muted">{$t("settings.storage.portable_macos_notice")}</p>{/if}
  </div>
{/if}

{#if failed}
  <SettingsEmptyState error title={$t("settings.dependencies.load_failed")} onRetry={() => void loadDeps()} />
{:else if loading && !loaded}
  <p class="muted" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
{:else if loaded && deps.length === 0}
  <SettingsEmptyState icon="blocks" title={$t("settings.advanced.components_empty")} hint={$t("settings.advanced.components_empty_hint")} onRetry={() => void loadDeps()} />
{/if}

{#if !failed && deps.length > 0}
  <div class="deps-wrap" data-setting-key="settings.dependencies.title">
    <table class="deps">
      <thead>
        <tr>
          <th scope="col">{$t("settings.dependencies.col_name")}</th>
          <th scope="col">{$t("settings.dependencies.col_status")}</th>
          <th scope="col" class="col-action"><span class="visually-hidden">{$t("settings.dependencies.col_action")}</span></th>
        </tr>
      </thead>
      <tbody>
        {#each deps as dep (dep.name)}
          <DependencyRow
            name={dep.name}
            installed={dep.installed}
            version={dep.version}
            source={dep.source}
            path={dep.path}
            outdated={dep.outdated}
            busy={installing === dep.name}
            error={rowErrors[dep.name] ?? null}
            onInstall={() => install(dep.name)}
            onAfterCustomFile={loadDeps}
          />
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .portable { padding-block: var(--space-2); margin-block-end: var(--space-2); border-block-end: 1px solid var(--separator); }
  .portable-label { margin: 0; font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .muted { margin: 2px 0 0; font-size: var(--text-sm); line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .deps-wrap { overflow-x: auto; max-inline-size: 100%; }
  .deps { inline-size: 100%; border-collapse: collapse; }
  .deps th {
    padding: var(--space-1) var(--space-2);
    font-size: var(--text-xs); font-weight: 500; color: var(--text-muted);
    text-align: start; border-block-end: 1px solid var(--separator);
  }
  .deps th:first-child { padding-inline-start: 0; }
  .deps .col-action { inline-size: 1%; }
  .deps :global(td) { padding: var(--space-2); vertical-align: middle; }
  .deps :global(td:first-child) { padding-inline-start: 0; }
  .deps :global(td:last-child) { padding-inline-end: 0; }
  .deps :global(tbody tr + tr td) { border-block-start: 1px solid var(--separator); }
  .visually-hidden { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
</style>
