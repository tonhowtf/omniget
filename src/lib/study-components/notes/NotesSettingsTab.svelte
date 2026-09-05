<script lang="ts">
  import { onMount } from "svelte";
  import {
    notesSettingsGet,
    notesSettingsSet,
    type NoteSettingValue,
  } from "$lib/notes-bridge";
  import MaintenancePanel from "./MaintenancePanel.svelte";

  type Props = {
    onToast: (kind: "ok" | "err", msg: string) => void;
  };

  let { onToast }: Props = $props();

  type Settings = {
    appearance_font_scale: number;
    appearance_reading_width: "compact" | "normal" | "wide";
    sidebar_width: number;
    sidebar_show_recents: boolean;
    sidebar_show_favorites: boolean;
    sidebar_show_today: boolean;
    editor_slash_menu: boolean;
    editor_drag_handle: boolean;
    editor_autosave_delay_ms: number;
    journal_default_template: "none" | "daily-journal" | "weekly-review" | "concept-page";
    journal_global_shortcut: boolean;
    refs_hover_delay_ms: number;
    refs_preview_count: number;
    advanced_op_log_enabled: boolean;
    advanced_debug_mode: boolean;
  };

  const DEFAULTS: Settings = {
    appearance_font_scale: 1.0,
    appearance_reading_width: "normal",
    sidebar_width: 280,
    sidebar_show_recents: true,
    sidebar_show_favorites: true,
    sidebar_show_today: true,
    editor_slash_menu: true,
    editor_drag_handle: true,
    editor_autosave_delay_ms: 500,
    journal_default_template: "none",
    journal_global_shortcut: true,
    refs_hover_delay_ms: 400,
    refs_preview_count: 5,
    advanced_op_log_enabled: true,
    advanced_debug_mode: false,
  };

  const KEY_MAP: Record<keyof Settings, string> = {
    appearance_font_scale: "appearance:font_scale",
    appearance_reading_width: "appearance:reading_width",
    sidebar_width: "sidebar:width",
    sidebar_show_recents: "sidebar:show_recents",
    sidebar_show_favorites: "sidebar:show_favorites",
    sidebar_show_today: "sidebar:show_today",
    editor_slash_menu: "editor:slash_menu",
    editor_drag_handle: "editor:drag_handle",
    editor_autosave_delay_ms: "editor:autosave_delay_ms",
    journal_default_template: "journal:default_template",
    journal_global_shortcut: "journal:global_shortcut",
    refs_hover_delay_ms: "refs:hover_delay_ms",
    refs_preview_count: "refs:preview_count",
    advanced_op_log_enabled: "advanced:op_log_enabled",
    advanced_debug_mode: "advanced:debug_mode",
  };

  let settings = $state<Settings>({ ...DEFAULTS });
  let loading = $state(true);
  let savingState = $state<"idle" | "saving" | "saved">("idle");
  let savedFlash: ReturnType<typeof setTimeout> | null = null;
  let saveTimers = new Map<keyof Settings, ReturnType<typeof setTimeout>>();

  function coerce<T>(value: NoteSettingValue, fallback: T): T {
    if (value === null || value === undefined) return fallback;
    if (typeof fallback === typeof value) return value as unknown as T;
    return fallback;
  }

  async function loadOne(key: keyof Settings) {
    try {
      const r = await notesSettingsGet(KEY_MAP[key]);
      const fallback = DEFAULTS[key] as NoteSettingValue;
      const next = coerce(r.value, fallback);
      (settings as Record<string, unknown>)[key] = next;
    } catch {
      (settings as Record<string, unknown>)[key] = DEFAULTS[key];
    }
  }

  async function loadAll() {
    loading = true;
    const keys = Object.keys(KEY_MAP) as (keyof Settings)[];
    await Promise.all(keys.map(loadOne));
    loading = false;
  }

  function flashSaved() {
    savingState = "saved";
    if (savedFlash) clearTimeout(savedFlash);
    savedFlash = setTimeout(() => (savingState = "idle"), 1200);
  }

  function patch<K extends keyof Settings>(key: K, value: Settings[K]) {
    settings[key] = value;
    savingState = "saving";
    const existing = saveTimers.get(key);
    if (existing) clearTimeout(existing);
    saveTimers.set(
      key,
      setTimeout(async () => {
        saveTimers.delete(key);
        try {
          await notesSettingsSet({
            key: KEY_MAP[key],
            value: value as NoteSettingValue,
          });
          flashSaved();
        } catch (e) {
          onToast("err", e instanceof Error ? e.message : String(e));
          savingState = "idle";
        }
      }, 500),
    );
  }

  onMount(() => {
    void loadAll();
    return () => {
      for (const t of saveTimers.values()) clearTimeout(t);
      if (savedFlash) clearTimeout(savedFlash);
    };
  });
</script>

<section class="tab">
  <header class="tab-head">
    <div>
      <h2>Notas</h2>
      <p class="hint">Aparência, sidebar, editor, journal, refs, manutenção.</p>
    </div>
    <div class="status">
      {#if loading}
        <span class="muted">Carregando…</span>
      {:else if savingState === "saving"}
        <span class="dot saving" aria-hidden="true"></span>
        <span>Salvando…</span>
      {:else if savingState === "saved"}
        <span class="dot saved" aria-hidden="true"></span>
        <span>Salvo</span>
      {/if}
    </div>
  </header>

  {#if !loading}
    <article class="card">
      <h3>Aparência</h3>
      <div class="rows">
        <label class="row">
          <span>Escala de fonte</span>
          <select
            value={String(settings.appearance_font_scale)}
            onchange={(e) =>
              patch(
                "appearance_font_scale",
                parseFloat((e.currentTarget as HTMLSelectElement).value),
              )}
          >
            <option value="0.95">95%</option>
            <option value="1">100% (padrão)</option>
            <option value="1.05">105%</option>
            <option value="1.1">110%</option>
          </select>
        </label>
        <label class="row">
          <span>Largura de leitura</span>
          <select
            value={settings.appearance_reading_width}
            onchange={(e) =>
              patch(
                "appearance_reading_width",
                (e.currentTarget as HTMLSelectElement).value as Settings["appearance_reading_width"],
              )}
          >
            <option value="compact">Compacta (640px)</option>
            <option value="normal">Normal (760px)</option>
            <option value="wide">Larga (920px)</option>
          </select>
        </label>
      </div>
    </article>

    <article class="card">
      <h3>Sidebar</h3>
      <div class="rows">
        <label class="row">
          <span>Largura padrão (px)</span>
          <select
            value={String(settings.sidebar_width)}
            onchange={(e) =>
              patch(
                "sidebar_width",
                parseInt((e.currentTarget as HTMLSelectElement).value, 10),
              )}
          >
            <option value="240">240</option>
            <option value="280">280</option>
            <option value="320">320</option>
            <option value="360">360</option>
          </select>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.sidebar_show_recents}
            onchange={(e) =>
              patch("sidebar_show_recents", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Mostrar seção "Recentes"</span>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.sidebar_show_favorites}
            onchange={(e) =>
              patch("sidebar_show_favorites", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Mostrar seção "Favoritas"</span>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.sidebar_show_today}
            onchange={(e) =>
              patch("sidebar_show_today", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Mostrar seção "Hoje"</span>
        </label>
      </div>
    </article>

    <article class="card">
      <h3>Editor</h3>
      <div class="rows">
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.editor_slash_menu}
            onchange={(e) =>
              patch("editor_slash_menu", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Slash menu (digite "/" pra abrir comandos)</span>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.editor_drag_handle}
            onchange={(e) =>
              patch("editor_drag_handle", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Drag handle visível ao passar o mouse</span>
        </label>
        <label class="row">
          <span>Atraso de auto-save</span>
          <select
            value={String(settings.editor_autosave_delay_ms)}
            onchange={(e) =>
              patch(
                "editor_autosave_delay_ms",
                parseInt((e.currentTarget as HTMLSelectElement).value, 10),
              )}
          >
            <option value="250">250ms (rápido)</option>
            <option value="500">500ms (padrão)</option>
            <option value="1000">1000ms (econômico)</option>
          </select>
        </label>
      </div>
    </article>

    <article class="card">
      <h3>Daily journal</h3>
      <div class="rows">
        <label class="row">
          <span>Template padrão ao criar journal</span>
          <select
            value={settings.journal_default_template}
            onchange={(e) =>
              patch(
                "journal_default_template",
                (e.currentTarget as HTMLSelectElement).value as Settings["journal_default_template"],
              )}
          >
            <option value="none">Nenhum (página vazia)</option>
            <option value="daily-journal">Daily Journal</option>
            <option value="weekly-review">Weekly Review</option>
            <option value="concept-page">Concept Page</option>
          </select>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.journal_global_shortcut}
            onchange={(e) =>
              patch("journal_global_shortcut", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Atalho global Ctrl+J abre o journal de hoje</span>
        </label>
      </div>
    </article>

    <article class="card">
      <h3>Refs &amp; backlinks</h3>
      <div class="rows">
        <label class="row">
          <span>Atraso ao passar o mouse em [[link]]</span>
          <select
            value={String(settings.refs_hover_delay_ms)}
            onchange={(e) =>
              patch(
                "refs_hover_delay_ms",
                parseInt((e.currentTarget as HTMLSelectElement).value, 10),
              )}
          >
            <option value="200">200ms (rápido)</option>
            <option value="400">400ms (padrão)</option>
            <option value="600">600ms (paciente)</option>
          </select>
        </label>
        <label class="row">
          <span>Blocos preview no popover</span>
          <select
            value={String(settings.refs_preview_count)}
            onchange={(e) =>
              patch(
                "refs_preview_count",
                parseInt((e.currentTarget as HTMLSelectElement).value, 10),
              )}
          >
            <option value="3">3</option>
            <option value="5">5 (padrão)</option>
            <option value="10">10</option>
          </select>
        </label>
      </div>
    </article>

    <article class="card">
      <h3>Avançado</h3>
      <div class="rows">
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.advanced_op_log_enabled}
            onchange={(e) =>
              patch("advanced_op_log_enabled", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Gravar op-log (necessário pra Ctrl+Z global e auditoria)</span>
        </label>
        <label class="row check">
          <input
            type="checkbox"
            checked={settings.advanced_debug_mode}
            onchange={(e) =>
              patch("advanced_debug_mode", (e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Modo debug (logs detalhados em devtools)</span>
        </label>
      </div>
    </article>

    <MaintenancePanel {onToast} />
  {/if}
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .tab-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .tab-head h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .hint {
    margin: 4px 0 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--secondary);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.saving {
    background: var(--warning);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .dot.saved {
    background: var(--success);
  }
  .muted {
    color: var(--tertiary);
  }
  .card {
    padding: 14px 16px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .card h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--secondary);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }
  .row > span {
    flex: 1 1 auto;
  }
  .row.check {
    justify-content: flex-start;
  }
  .row.check > span {
    flex: 0 1 auto;
  }
  .row select {
    min-width: 200px;
    padding: 6px 8px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .row input[type="checkbox"] {
    width: 16px;
    height: 16px;
    margin: 0;
    accent-color: var(--accent);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }
</style>
