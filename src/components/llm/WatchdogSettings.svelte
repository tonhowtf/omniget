<script lang="ts">
  /**
   * Stuck agents and approvals: the turn watchdog, the job time limit, how
   * long a permission question waits, and when it becomes a notification.
   *
   * The backend stores seconds for the turn windows and the questions and
   * clamps every value (`commands/llm/watchdog.rs`), so a zero can never kill
   * every turn or deny every question; this card shows minutes where people
   * think in minutes and sends each field on its own as soon as it changes.
   * What comes back from the backend (clamped) is what the card shows.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  type Config = {
    turn_idle_timeout_s: number;
    turn_tool_idle_timeout_s: number;
    turn_start_timeout_s: number;
    job_default_deadline_min: number;
    ask_timeout_chat_s: number;
    /** null = until the job's own time limit. */
    ask_timeout_job_s: number | null;
    approval_notify_delay_s: number;
    /** Resume by itself after a restart (3.1); off by default. */
    auto_resume: boolean;
  };

  /**
   * One row: its key in the settings, the i18n stem, and `minutes` when the
   * backend stores seconds but the person types minutes (the label says the
   * unit; the deadline is already stored in minutes).
   */
  type Field = {
    key: Exclude<keyof Config, "auto_resume">;
    stem: string;
    minutes: boolean;
    min: number;
    allowZero?: boolean;
  };

  const FIELDS: Field[] = [
    { key: "turn_idle_timeout_s", stem: "idle", minutes: true, min: 1 },
    { key: "turn_tool_idle_timeout_s", stem: "tool_idle", minutes: true, min: 1 },
    { key: "turn_start_timeout_s", stem: "start", minutes: false, min: 10 },
    { key: "job_default_deadline_min", stem: "deadline", minutes: false, min: 1 },
    { key: "ask_timeout_chat_s", stem: "ask_chat", minutes: true, min: 1 },
    { key: "ask_timeout_job_s", stem: "ask_job", minutes: true, min: 0, allowZero: true },
    { key: "approval_notify_delay_s", stem: "notify_delay", minutes: false, min: 0, allowZero: true },
  ];

  let config = $state<Config | null>(null);
  let offline = $state(false);
  let busy = $state(false);

  /** What the input shows: minutes for the fields stored in seconds. */
  function shown(field: Field, c: Config): number {
    const raw = c[field.key];
    if (raw === null) return 0;
    return field.minutes ? Math.round(raw / 60) : raw;
  }

  async function refresh() {
    try {
      config = await invoke<Config>("llm_watchdog_get");
      offline = false;
    } catch {
      offline = true;
    }
  }

  onMount(refresh);

  async function saveAutoResume(on: boolean) {
    if (busy) return;
    busy = true;
    try {
      config = await invoke<Config>("llm_watchdog_set", { config: { auto_resume: on } });
      offline = false;
    } catch {
      offline = true;
    } finally {
      busy = false;
    }
  }

  async function save(field: Field, value: number) {
    if (busy || !Number.isFinite(value)) return;
    const typed = Math.max(field.allowZero ? 0 : field.min, Math.round(value));
    const stored = field.minutes ? typed * 60 : typed;
    busy = true;
    try {
      config = await invoke<Config>("llm_watchdog_set", { config: { [field.key]: stored } });
      offline = false;
    } catch {
      offline = true;
    } finally {
      busy = false;
    }
  }
</script>

<section class="watchdog">
  <div class="group">
    <div class="group-row stack">
      <div class="group-row-title">{$t("llm.watchdog.title")}</div>
      <div class="group-row-sub">{$t("llm.watchdog.subtitle")}</div>
    </div>
    {#if config}
      {#each FIELDS as field (field.key)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t(`llm.watchdog.${field.stem}`)}</div>
            <div class="group-row-sub">{$t(`llm.watchdog.${field.stem}_desc`)}</div>
          </div>
          <input
            class="input number"
            type="number"
            min={field.allowZero ? 0 : field.min}
            value={shown(field, config)}
            disabled={busy}
            onchange={(e) => void save(field, Number((e.currentTarget as HTMLInputElement).value))}
            aria-label={$t(`llm.watchdog.${field.stem}`)}
          />
        </div>
      {/each}
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.watchdog.auto_resume")}</div>
          <div class="group-row-sub">{$t("llm.watchdog.auto_resume_desc")}</div>
        </div>
        <button
          type="button"
          class="toggle"
          class:on={config.auto_resume}
          role="switch"
          aria-checked={config.auto_resume}
          aria-label={$t("llm.watchdog.auto_resume")}
          disabled={busy}
          onclick={() => config && void saveAutoResume(!config.auto_resume)}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
    {/if}
    {#if offline}
      <p class="hint danger">{$t("llm.watchdog.offline")}</p>
    {/if}
  </div>
</section>

<style>
  .watchdog {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin-top: var(--space-4);
  }

  .stack {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-1);
  }

  .number {
    width: 9ch;
    text-align: right;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .danger {
    color: var(--danger);
  }

  @media (max-width: 720px) {
    .watchdog :global(.group-row) {
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>
