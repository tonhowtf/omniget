<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

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
    try {
      stats = await invoke<StoreStats>("content_store_stats");
    } catch {
      stats = null;
    }
  }

  async function loadPortable() {
    try {
      portable = await invoke<PortableInfo>("get_portable_info");
    } catch {
      portable = null;
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

  $effect(() => {
    void loadStats();
    void loadPortable();
  });
</script>

<section class="section">
  <h5 class="section-title">{$t("settings.storage.title")}</h5>
  <p class="muted">{$t("settings.storage.description")}</p>

  {#if portable?.is_portable}
    <div class="card portable-card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.storage.portable_label")}</span>
          <span class="setting-path wrap">
            {$t("settings.storage.portable_active", { dir: portable.data_dir ?? "<app>/data" })}
          </span>
        </div>
      </div>

      <!-- O modo portatil nao cobre o WebView no macOS, e o usuario que leva o
           pendrive para outra maquina precisa saber disso antes, nao depois. -->
      {#if portable.macos_webview_notice}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-path wrap">{$t("settings.storage.portable_macos_notice")}</span>
          </div>
        </div>
      {/if}
    </div>
  {/if}

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("settings.storage.dedupe_label")}</span>
        <span class="setting-path">
          {#if stats && stats.objects > 0 && stats.savings_measurable}
            {$t("settings.storage.dedupe_stats", {
              objects: stats.objects,
              saved: humanBytes(saved),
            })}
          {:else if stats && stats.objects > 0}
            <!-- Windows não sabe contar links. Dizer "0 economizado" seria dar
                 cara de medição a algo que não foi medido. -->
            {$t("settings.storage.dedupe_unmeasurable", { objects: stats.objects })}
          {:else}
            {$t("settings.storage.dedupe_empty")}
          {/if}
        </span>
      </div>
      <button type="button" class="button" disabled={running} onclick={runDedupe}>
        {running ? $t("settings.storage.dedupe_running") : $t("settings.storage.dedupe_action")}
      </button>
    </div>

    <div class="divider"></div>

    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("settings.storage.silence_label")}</span>
        <span class="setting-path">{$t("settings.storage.silence_hint")}</span>
      </div>
      <button type="button" class="button" disabled={probing} onclick={probeSilence}>
        {probing ? $t("settings.storage.silence_running") : $t("settings.storage.silence_action")}
      </button>
    </div>
  </div>
</section>

<style>
  .portable-card {
    margin-bottom: var(--padding);
  }

  /* O caminho de dados e o aviso do macOS sao frases inteiras, nao rotulos:
     sem isto ficam numa linha so e sao cortados. */
  .wrap {
    white-space: normal;
    word-break: break-word;
    line-height: 1.5;
  }
</style>
