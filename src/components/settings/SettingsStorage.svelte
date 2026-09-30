<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

  type StoreStats = {
    objects: number;
    bytes_on_disk: number;
    bytes_without_dedupe: number;
    savings_measurable: boolean;
  };

  type DedupeReport = {
    examined: number;
    deduplicated: number;
    bytes_saved: number;
    savings_measurable: boolean;
    errors: string[];
  };

  type SilenceMapInfo = {
    savings_secs: number;
    from_cache: boolean;
    map: { media_duration_secs: number; spans: unknown[] };
  };

  type PortableInfo = {
    is_portable: boolean;
    data_dir: string | null;
    macos_webview_notice: boolean;
  };

  let stats = $state<StoreStats | null>(null);
  let portable = $state<PortableInfo | null>(null);
  let statsLoading = $state(true);
  let statsError = $state(false);
  let portableError = $state(false);
  let running = $state(false);
  let probing = $state(false);

  function humanBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1);
    return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function humanSecs(s: number): string {
    const total = Math.round(s);
    const m = Math.floor(total / 60);
    const sec = total % 60;
    if (m === 0) return `${sec}s`;
    return `${m}min ${sec}s`;
  }

  // O que o dedupe já evitou de ocupar. Cada link extra é uma cópia que não foi
  // feita — se for zero, dizemos zero em vez de esconder a linha.
  let saved = $derived(
    stats ? Math.max(0, stats.bytes_without_dedupe - stats.bytes_on_disk) : 0,
  );

  async function loadStats() {
    statsLoading = true;
    statsError = false;
    try {
      stats = await invoke<StoreStats>("content_store_stats");
    } catch {
      statsError = true;
    } finally {
      statsLoading = false;
    }
  }

  async function loadPortable() {
    portableError = false;
    try {
      portable = await invoke<PortableInfo>("get_portable_info");
    } catch {
      portableError = true;
    }
  }

  async function runDedupe() {
    const chosen = await open({
      multiple: true,
      title: $t("settings.storage.pick_files") as string,
    });
    const paths = Array.isArray(chosen) ? chosen : chosen ? [chosen] : [];
    if (paths.length === 0) return;

    running = true;
    try {
      const rep = await invoke<DedupeReport>("deduplicate_files", { paths });
      if (rep.deduplicated === 0) {
        showToast("info", $t("settings.storage.dedupe_nothing", { examined: rep.examined }) as string);
      } else if (rep.savings_measurable) {
        showToast(
          "success",
          $t("settings.storage.dedupe_done", {
            count: rep.deduplicated,
            size: humanBytes(rep.bytes_saved),
          }) as string,
        );
      } else {
        // Sem medição, dizer quantos foram ligados é verdade; dizer quanto
        // economizou não seria.
        showToast(
          "success",
          $t("settings.storage.dedupe_done_unmeasured", { count: rep.deduplicated }) as string,
        );
      }
      // Erros não podem sumir: um arquivo ilegível é exatamente o que o usuário
      // precisa saber que ficou de fora.
      if (rep.errors.length > 0) {
        showToast("error", $t("settings.storage.dedupe_errors", { count: rep.errors.length }) as string);
      }
      void loadStats();
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  async function probeSilence() {
    const chosen = await open({
      multiple: false,
      title: $t("settings.storage.pick_media") as string,
    });
    if (typeof chosen !== "string") return;

    probing = true;
    try {
      const info = await invoke<SilenceMapInfo>("compute_silence_map", { path: chosen, force: false });
      if (info.savings_secs < 1) {
        showToast("info", $t("settings.storage.silence_none") as string);
      } else {
        showToast(
          "success",
          $t("settings.storage.silence_done", { saved: humanSecs(info.savings_secs) }) as string,
        );
      }
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      probing = false;
    }
  }

  onMount(() => {
    void loadStats();
    void loadPortable();
  });
</script>

<div class="storage" data-setting-key="settings.storage.title">
  {#if portableError}
    <SettingsEmptyState error title={$t("settings.storage.portable_load_failed")} onRetry={() => void loadPortable()} />
  {/if}
  {#if portable?.is_portable}
    <div class="portable" data-setting-key="settings.storage.portable_label">
      <p class="portable-label">{$t("settings.storage.portable_label")}</p>
      <p class="muted wrap">{$t("settings.storage.portable_active", { dir: portable.data_dir ?? "<app>/data" })}</p>
      <!-- O modo portatil nao cobre o WebView no macOS, e o usuario que leva o
           pendrive para outra maquina precisa saber disso antes, nao depois. -->
      {#if portable.macos_webview_notice}
        <p class="muted wrap">{$t("settings.storage.portable_macos_notice")}</p>
      {/if}
    </div>
  {/if}

  <SettingRow id="settings-storage-dedupe" label={$t("settings.storage.dedupe_label")} settingKey="settings.storage.dedupe_label"
    help={statsLoading ? $t("settings.rebrand.state.loading")
      : statsError ? undefined
      : stats && stats.objects > 0 && stats.savings_measurable ? $t("settings.storage.dedupe_stats", { objects: stats.objects, saved: humanBytes(saved) })
      : stats && stats.objects > 0 ? $t("settings.storage.dedupe_unmeasurable", { objects: stats.objects })
      : $t("settings.storage.dedupe_empty")}>
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={running} onclick={runDedupe}>
        {running ? $t("settings.storage.dedupe_running") : $t("settings.storage.dedupe_action")}
      </button>
    {/snippet}
  </SettingRow>
  {#if statsError}
    <!-- Windows não sabe contar links; uma leitura que falhou nunca vira "0 economizado". -->
    <SettingsEmptyState error title={$t("settings.storage.stats_load_failed")} onRetry={() => void loadStats()} />
  {/if}

  <SettingRow id="settings-storage-silence" label={$t("settings.storage.silence_label")} help={$t("settings.storage.silence_hint")} settingKey="settings.storage.silence_label">
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={probing} onclick={probeSilence}>
        {probing ? $t("settings.storage.silence_running") : $t("settings.storage.silence_action")}
      </button>
    {/snippet}
  </SettingRow>
</div>

<style>
  .storage { display: flex; flex-direction: column; min-inline-size: 0; }
  .portable { padding-block: var(--space-2); border-block-end: 1px solid var(--separator); }
  .portable-label { margin: 0; font-size: 13px; line-height: 20px; color: var(--text); }
  .muted { margin: 2px 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }

  /* O caminho de dados e o aviso do macOS sao frases inteiras, nao rotulos:
     sem isto ficam numa linha so e sao cortados. */
  .wrap {
    white-space: normal;
    word-break: break-word;
    line-height: 1.5;
  }
</style>
