<script lang="ts">
  /**
   * Ferramenta "Spotify → Spicetify". O backend chama o CLI oficial; esta
   * tela mostra o estado (CLI, Spotify, aplicado ou não), troca tema e
   * esquema, instala o Marketplace e lista extensões e apps.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type ThemeInfo = { name: string; schemes: string[] };
  type Status = {
    installed: boolean;
    version: string | null;
    source: string;
    path: string | null;
    config_dir: string | null;
    config_path: string | null;
    spotify_path: string | null;
    spotify_error: string | null;
    applied: boolean;
    current_theme: string;
    color_scheme: string;
    extensions: string[];
    custom_apps: string[];
    marketplace_installed: boolean;
    themes: ThemeInfo[];
    flatpak: boolean;
  };
  type CmdOutput = { ok: boolean; code: number | null; stdout: string; stderr: string };

  let status = $state<Status | null>(null);
  let loading = $state(true);
  let busy = $state<string | null>(null);
  let log = $state("");
  let themeSel = $state("");
  let schemeSel = $state("");

  let schemes = $derived(status?.themes.find((th) => th.name === themeSel)?.schemes ?? []);
  let spotifyOk = $derived(!!status?.spotify_path);
  let canAct = $derived(!!status?.installed && spotifyOk && !status?.flatpak && busy === null);

  function errText(e: unknown): string {
    return typeof e === "string" ? e : ((e as any)?.message ?? String(e));
  }

  async function refresh() {
    try {
      status = (await invoke<Status | null>("spicetify_status")) ?? null;
      themeSel = status?.current_theme ?? "";
      schemeSel = status?.color_scheme ?? "";
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      loading = false;
    }
  }

  async function runAction(id: string, fn: () => Promise<unknown>) {
    if (busy) return;
    busy = id;
    try {
      const res = await fn();
      const out = res as CmdOutput | string | undefined;
      if (out && typeof out === "object" && "stdout" in out) {
        log = [out.stdout, out.stderr].filter((s) => s && s.trim()).join("\n").trim();
      } else if (typeof out === "string") {
        log = out;
      }
      showToast("success", $t("tools.spicetify.done") as string);
    } catch (e) {
      log = errText(e);
      showToast("error", $t("tools.spicetify.failed") as string);
    } finally {
      busy = null;
      await refresh();
    }
  }

  const install = () => runAction("install", () => invoke("spicetify_install"));
  const action = (a: string) => runAction(a, () => invoke("spicetify_action", { action: a }));
  const applyTheme = () =>
    runAction("theme", () => invoke("spicetify_set_theme", { theme: themeSel, scheme: schemeSel }));
  const installMarketplace = () => runAction("marketplace", () => invoke("spicetify_install_marketplace"));
  const removeAddon = (kind: "extension" | "custom_app", name: string) =>
    runAction(`remove:${name}`, () => invoke("spicetify_remove_addon", { kind, name }));

  async function openSpotifyDownload() {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl("https://www.spotify.com/download/");
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function onThemeChange() {
    schemeSel = schemes[0] ?? "";
  }

  function sourceLabel(src: string): string {
    if (src === "managed") return $t("tools.spicetify.cli_source_managed") as string;
    if (src === "custom") return $t("tools.spicetify.cli_source_custom") as string;
    return $t("tools.spicetify.cli_source_system") as string;
  }

  onMount(refresh);
</script>

<div class="spicetify">
  {#if status?.flatpak}
    <div class="notice warn">{$t("tools.spicetify.flatpak")}</div>
  {/if}

  <!-- Estado -->
  <section>
    <span class="group-label">{$t("tools.spicetify.title")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.cli_row")}</div>
          <div class="group-row-sub">
            {#if loading}
              …
            {:else if status?.installed}
              v{status.version ?? "?"} · {sourceLabel(status.source)}
            {:else}
              {$t("tools.spicetify.cli_missing")}
            {/if}
          </div>
        </div>
        <div class="group-row-trailing">
          {#if !loading && !status?.installed}
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null || status?.flatpak} onclick={install}>
              {busy === "install" ? $t("tools.spicetify.installing") : $t("tools.spicetify.install")}
            </button>
          {:else if status?.installed}
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => action("upgrade")}>
              {busy === "upgrade" ? $t("tools.spicetify.upgrading") : $t("tools.spicetify.upgrade")}
            </button>
          {/if}
        </div>
      </div>

      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.spotify_row")}</div>
          <div class="group-row-sub">
            {#if !status?.installed}
              {$t("tools.spicetify.install_first")}
            {:else if spotifyOk}
              <span class="mono">{status?.spotify_path}</span>
            {:else}
              <span class="danger">{$t("tools.spicetify.spotify_missing")}</span>
              · {$t("tools.spicetify.spotify_hint")}
            {/if}
          </div>
        </div>
        {#if status?.installed && !spotifyOk}
          <div class="group-row-trailing">
            <button class="btn btn-secondary btn-sm" type="button" onclick={openSpotifyDownload}>
              {$t("tools.spicetify.spotify_download")}
            </button>
          </div>
        {/if}
      </div>

      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.state_row")}</div>
          <div class="group-row-sub">
            {#if status?.applied}
              <span class="tag tag-success">{$t("tools.spicetify.state_applied")}</span>
            {:else}
              <span class="tag">{$t("tools.spicetify.state_original")}</span>
              {#if canAct}
                · {$t("tools.spicetify.first_apply_hint")}
              {/if}
            {/if}
          </div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if status?.applied}
            <button class="btn btn-secondary btn-sm" type="button" disabled={!canAct} onclick={() => action("apply")}>
              {busy === "apply" ? $t("tools.spicetify.working") : $t("tools.spicetify.apply")}
            </button>
            <button class="btn btn-destructive btn-sm" type="button" disabled={!canAct} onclick={() => action("restore")}>
              {busy === "restore" ? $t("tools.spicetify.working") : $t("tools.spicetify.restore")}
            </button>
          {:else}
            <button class="btn btn-primary btn-sm" type="button" disabled={!canAct} onclick={() => action("backup_apply")}>
              {busy === "backup_apply" ? $t("tools.spicetify.working") : $t("tools.spicetify.apply")}
            </button>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <!-- Aparência -->
  <section>
    <span class="group-label">{$t("tools.spicetify.appearance")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.theme")}</div>
          {#if status?.installed && status.themes.length === 0}
            <div class="group-row-sub">{$t("tools.spicetify.no_themes")}</div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <select bind:value={themeSel} onchange={onThemeChange} disabled={!status?.installed || busy !== null}>
            <option value="">{$t("tools.spicetify.theme_default")}</option>
            {#each status?.themes ?? [] as th (th.name)}
              <option value={th.name}>{th.name}</option>
            {/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.scheme")}</div>
        </div>
        <div class="group-row-trailing">
          <select bind:value={schemeSel} disabled={!themeSel || schemes.length === 0 || busy !== null}>
            {#if schemes.length === 0}
              <option value="">—</option>
            {/if}
            {#each schemes as sc (sc)}
              <option value={sc}>{sc}</option>
            {/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing">
          <button class="btn btn-primary btn-sm" type="button" disabled={!canAct} onclick={applyTheme}>
            {busy === "theme" ? $t("tools.spicetify.working") : $t("tools.spicetify.apply_theme")}
          </button>
        </div>
      </div>
    </div>
  </section>

  <!-- Marketplace -->
  <section>
    <span class="group-label">{$t("tools.spicetify.marketplace")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.marketplace")}</div>
          <div class="group-row-sub">
            {status?.marketplace_installed ? $t("tools.spicetify.marketplace_hint") : $t("tools.spicetify.marketplace_desc")}
          </div>
        </div>
        <div class="group-row-trailing">
          {#if status?.marketplace_installed}
            <span class="tag tag-success">{$t("tools.spicetify.marketplace_installed")}</span>
          {:else}
            <button class="btn btn-primary btn-sm" type="button" disabled={!canAct} onclick={installMarketplace}>
              {busy === "marketplace" ? $t("tools.spicetify.working") : $t("tools.spicetify.marketplace_install")}
            </button>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <!-- Extensões e apps -->
  <section>
    <span class="group-label">{$t("tools.spicetify.addons")}</span>
    <div class="group">
      {#if (status?.extensions.length ?? 0) === 0 && (status?.custom_apps.length ?? 0) === 0}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-sub">{$t("tools.spicetify.addons_empty")}</div>
          </div>
        </div>
      {/if}
      {#each status?.extensions ?? [] as ext (ext)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title mono">{ext}</div>
            <div class="group-row-sub">{$t("tools.spicetify.extensions")}</div>
          </div>
          <div class="group-row-trailing">
            <button class="btn btn-ghost btn-sm" type="button" disabled={!canAct} onclick={() => removeAddon("extension", ext)}>
              {$t("tools.spicetify.remove")}
            </button>
          </div>
        </div>
      {/each}
      {#each status?.custom_apps ?? [] as app (app)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title mono">{app}</div>
            <div class="group-row-sub">{$t("tools.spicetify.custom_apps")}</div>
          </div>
          <div class="group-row-trailing">
            <button class="btn btn-ghost btn-sm" type="button" disabled={!canAct} onclick={() => removeAddon("custom_app", app)}>
              {$t("tools.spicetify.remove")}
            </button>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <!-- Avançado -->
  <section>
    <span class="group-label">{$t("tools.spicetify.advanced")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.spicetify.block_updates")}</div>
          <div class="group-row-sub">{$t("tools.spicetify.block_updates_desc")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" disabled={!canAct} onclick={() => action("block_updates")}>
            {$t("tools.spicetify.block_updates")}
          </button>
          <button class="btn btn-ghost btn-sm" type="button" disabled={!canAct} onclick={() => action("unblock_updates")}>
            {$t("tools.spicetify.unblock_updates")}
          </button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing btn-row wrap">
          <button class="btn btn-secondary btn-sm" type="button" disabled={!canAct} onclick={() => action("refresh")}>{$t("tools.spicetify.refresh")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={!canAct} onclick={() => action("restart")}>{$t("tools.spicetify.restart")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={!status?.installed || busy !== null} onclick={() => action("open_config_dir")}>{$t("tools.spicetify.open_config")}</button>
          <button class="btn btn-secondary btn-sm" type="button" disabled={!canAct} onclick={() => action("enable_devtools")}>{$t("tools.spicetify.devtools")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if log}
    <details class="log" open>
      <summary>{$t("tools.spicetify.log")}</summary>
      <pre>{log}</pre>
    </details>
  {/if}
</div>

<style>
  .spicetify {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  section {
    display: flex;
    flex-direction: column;
  }

  .notice {
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .notice.warn {
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    color: var(--warning);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .danger {
    color: var(--danger);
    font-weight: 500;
  }

  .btn-row {
    display: flex;
    gap: var(--space-2);
  }

  .btn-row.wrap {
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .group-row-trailing select {
    min-width: 180px;
  }

  .log summary {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 var(--space-2);
  }

  .log pre {
    margin: var(--space-2) 0 0;
    padding: var(--space-3) var(--space-4);
    max-height: 260px;
    overflow: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
