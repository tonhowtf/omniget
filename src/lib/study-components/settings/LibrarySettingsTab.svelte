<script lang="ts">
  import SettingsField from "./SettingsField.svelte";
  import SettingsSlider from "./SettingsSlider.svelte";
  import SettingsToggle from "./SettingsToggle.svelte";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import type { StudySettings } from "$lib/study-bridge";

  type Props = {
    settings: StudySettings;
    onPatch: (patch: StudySettings) => void;
  };

  let { settings, onPatch }: Props = $props();
  const library = $derived(settings.library ?? {});
  let rescanning = $state(false);
  let rescanReport = $state<string | null>(null);

  function setLibrary<K extends keyof NonNullable<StudySettings["library"]>>(
    key: K,
    value: NonNullable<StudySettings["library"]>[K],
  ) {
    onPatch({ library: { ...(settings.library ?? {}), [key]: value } });
  }

  async function rescan() {
    rescanning = true;
    rescanReport = null;
    try {
      const r = await pluginInvoke<{
        courses_found: number;
        lessons_found: number;
        ms: number;
        new_lessons_notified?: number;
      }>("study", "study:rescan");
      rescanReport = `${r.courses_found} cursos · ${r.lessons_found} aulas · ${r.new_lessons_notified ?? 0} aulas novas detectadas (${r.ms}ms)`;
    } catch (e) {
      rescanReport = e instanceof Error ? e.message : String(e);
    } finally {
      rescanning = false;
    }
  }
</script>

<section class="tab">
  <SettingsField
    label="Watcher ativado"
    description={$t("study.settings.library.watch_desc")}
  >
    <SettingsToggle
      value={library.watcher_enabled ?? true}
      onChange={(v) => setLibrary("watcher_enabled", v)}
      ariaLabel="Watcher"
    />
  </SettingsField>

  <SettingsField
    label="Incluir pastas ocultas"
    description="Inclui pastas com nome iniciado em ponto durante o scan"
  >
    <SettingsToggle
      value={library.scan_hidden ?? false}
      onChange={(v) => setLibrary("scan_hidden", v)}
      ariaLabel="Pastas ocultas"
    />
  </SettingsField>

  <SettingsField
    label={$t("study.settings.library.auto_cleanup")}
    description={$t("study.settings.library.auto_cleanup_desc")}
  >
    <SettingsToggle
      value={library.auto_vacuum ?? true}
      onChange={(v) => setLibrary("auto_vacuum", v)}
      ariaLabel={$t("study.settings.library.auto_cleanup")}
    />
  </SettingsField>

  <SettingsField
    label="Intervalo de limpeza"
    description="Quantos dias entre cada vacuum"
    valueDisplay={`${library.auto_vacuum_interval_days ?? 30}d`}
  >
    <SettingsSlider
      value={library.auto_vacuum_interval_days ?? 30}
      min={7}
      max={90}
      step={1}
      onChange={(v) => setLibrary("auto_vacuum_interval_days", v)}
    />
  </SettingsField>

  <div class="actions">
    <div>
      <strong>Re-scanear biblioteca</strong>
      <p class="hint">Força detecção de novos cursos/aulas e dispara notificações</p>
      {#if rescanReport}
        <p class="report">{rescanReport}</p>
      {/if}
    </div>
    <button type="button" class="btn" disabled={rescanning} onclick={rescan}>
      {rescanning ? "Escaneando…" : "Re-scanear agora"}
    </button>
  </div>
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }

  .actions {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 16px;
    margin-top: 16px;
    background: color-mix(in oklab, var(--accent) 4%, transparent);
    border: none;
    border-radius: 8px;
  }

  .hint {
    margin: 4px 0 0;
    font-size: 12px;
    color: color-mix(in oklab, currentColor 60%, transparent);
  }

  .report {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--accent);
  }

  .btn {
    padding: 8px 16px;
    background: var(--accent);
    color: var(--accent-contrast, white);
    border: 1px solid var(--accent);
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    flex: 0 0 auto;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
