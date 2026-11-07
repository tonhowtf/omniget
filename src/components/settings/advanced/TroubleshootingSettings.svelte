<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import type { ResetScope } from "$lib/stores/settings-v3-types";
  import { isDebugEnabled, setDebugEnabled } from "$lib/stores/debug-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingsIcon from "../rebrand/SettingsIcon.svelte";

  let { askReset }: { askReset: (scope: ResetScope) => void } = $props();

  type Report = { path: string; shownPath: string; files: string[]; logLines: number; mailto: string; issueUrl: string };
  let report = $state<Report | null>(null);
  let reporting = $state(false);
  let reportError = $state<string | null>(null);

  function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  async function makeReport() {
    if (reporting) return;
    reporting = true;
    reportError = null;
    try {
      report = await invoke<Report>("canvas_report_problem", { reveal: true });
    } catch (e) {
      reportError = errorText(e);
    } finally {
      reporting = false;
    }
  }

  async function openLink(url: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
    } catch (e) {
      showToast("error", errorText(e));
    }
  }
</script>

<SettingRow id="setting-debug-enable" label={$t("debug.enable")} help={$t("debug.enable_desc")} settingKey="debug.enable">
  {#snippet control({ id, describedBy })}
    <button {id} type="button" class="toggle" class:on={isDebugEnabled()} role="switch" aria-checked={isDebugEnabled()} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
      onclick={() => setDebugEnabled(!isDebugEnabled())}><span class="toggle-knob"></span></button>
  {/snippet}
</SettingRow>
{#if isDebugEnabled()}
  <p class="follow">
    <a class="link" href="/community/debug">{$t("settings.advanced.activity_open")}</a>
  </p>
{/if}

<SettingRow id="setting-report-problem" label={$t("settings.advanced.report_title")} help={$t("settings.advanced.report_desc")} settingKey="settings.advanced.report_title">
  {#snippet control({ id, describedBy })}
    <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={reporting} onclick={makeReport}>
      {reporting ? $t("settings.advanced.report_running") : $t("settings.advanced.report_action")}
    </button>
  {/snippet}
</SettingRow>
{#if reportError}
  <p class="state error" role="alert">
    <SettingsIcon name="alert" size={18} />
    <span><bdi>{$t("settings.advanced.report_failed", { msg: reportError })}</bdi></span>
  </p>
{:else if report}
  <div class="state done" role="status">
    <SettingsIcon name="check" size={18} />
    <div class="done-text">
      <span>{$t("settings.advanced.report_saved")}</span>
      <code dir="ltr">{report.shownPath}</code>
      <span class="muted">{$t("settings.advanced.report_files", { count: report.files.length, lines: report.logLines })}</span>
      <span class="actions">
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => openLink(report!.mailto)}>{$t("settings.advanced.report_mail")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => openLink(report!.issueUrl)}>{$t("settings.advanced.report_issue")}</button>
      </span>
    </div>
  </div>
{/if}

<SettingRow id="settings-reset-section" label={$t("settings.rebrand.reset.section_action")} help={$t("settings.rebrand.reset.section_body")} settingKey="settings.rebrand.reset.section_action">
  {#snippet control({ id, describedBy })}
    <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={() => askReset({ kind: "section", section: "advanced" })}>
      {$t("settings.rebrand.reset.section_action")}
    </button>
  {/snippet}
</SettingRow>
<SettingRow id="settings-reset-app" label={$t("settings.rebrand.reset.app_action")} help={$t("settings.rebrand.reset.app_body")} settingKey="settings.advanced.reset">
  {#snippet control({ id, describedBy })}
    <button {id} type="button" class="btn btn-destructive btn-sm" aria-describedby={describedBy} onclick={() => askReset({ kind: "app" })}>
      {$t("settings.rebrand.reset.app_action")}
    </button>
  {/snippet}
</SettingRow>

<style>
  .follow { margin: 0 0 var(--space-2); }
  .link { display: inline-flex; align-items: center; gap: 2px; color: var(--accent-text, var(--accent)); font-size: var(--text-sm); text-decoration: none; border-radius: var(--radius-xs); }
  .link:hover { text-decoration: underline; }
  .link:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .state { display: flex; align-items: flex-start; gap: var(--space-2); margin: 0 0 var(--space-2); font-size: var(--text-sm); line-height: var(--leading-sm, 18px); color: var(--text); }
  .state.error { color: var(--error); }
  .state.done :global(svg) { color: var(--success); flex: none; }
  .done-text { display: flex; flex-direction: column; gap: var(--space-1); min-inline-size: 0; }
  .done-text code { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); word-break: break-all; }
  .muted { color: var(--text-muted); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
</style>
