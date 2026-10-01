<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    canvasPrefs,
    replaceCanvasPrefs,
    watchCanvasPrefs,
    type AgentPreset,
    type CanvasPrefs,
  } from "$lib/canvas/prefs.svelte";
  import { TERM_THEME_IDS } from "$lib/canvas/term/themes";
  import { SHORTCUT_ACTIONS, normCombo } from "$lib/canvas/interaction/keyboard";
  import { onMount } from "svelte";
  import SettingsCanvasEnvironments from "./SettingsCanvasEnvironments.svelte";
  import SettingsCanvasImages from "./SettingsCanvasImages.svelte";

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  const prefs = $derived(canvasPrefs());
  let busy = $state(false);

  onMount(() => watchCanvasPrefs());

  async function save(mutate: (c: CanvasPrefs) => void) {
    if (busy) return;
    busy = true;
    try {
      const next = structuredClone($state.snapshot(prefs)) as CanvasPrefs;
      mutate(next);
      await replaceCanvasPrefs(next);
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  const BUILTIN = ["shell", "claude", "codex", "gemini", "opencode", "ollama"];

  const SHORTCUT_IDS = Object.keys(SHORTCUT_ACTIONS);
  const isMacOs = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  let recording = $state<string | null>(null);

  function comboLabel(combo: string): string {
    if (!combo) return tr("canvas.settings.shortcutNone", "None");
    const names: Record<string, string> = isMacOs
      ? { mod: "⌘", alt: "⌥", shift: "⇧", arrowright: "→", arrowleft: "←", arrowup: "↑", arrowdown: "↓" }
      : { mod: "Ctrl+", alt: "Alt+", shift: "Shift+", arrowright: "→", arrowleft: "←", arrowup: "↑", arrowdown: "↓" };
    return combo
      .split("+")
      .map((p) => names[p] ?? p.toUpperCase())
      .join("");
  }

  function currentCombo(id: string): string {
    const o = prefs.shortcuts?.[id];
    return normCombo(o ?? SHORTCUT_ACTIONS[id].combo);
  }

  function recordKey(id: string, e: KeyboardEvent) {
    if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = null;
      return;
    }
    let combo = "";
    if (e.key !== "Backspace" && e.key !== "Delete") {
      const parts: string[] = [];
      if (isMacOs ? e.metaKey : e.ctrlKey) parts.push("mod");
      if (e.altKey) parts.push("alt");
      if (e.shiftKey) parts.push("shift");
      parts.push(e.key.toLowerCase());
      combo = normCombo(parts.join("+"));
    }
    recording = null;
    void save((c) => {
      const map: Record<string, string> = { ...(c.shortcuts ?? {}) };
      if (combo) for (const other of SHORTCUT_IDS) if (other !== id && currentCombo(other) === combo) map[other] = "";
      map[id] = combo;
      c.shortcuts = map;
    });
  }

  function resetShortcut(id: string) {
    void save((c) => {
      const map: Record<string, string> = { ...(c.shortcuts ?? {}) };
      delete map[id];
      c.shortcuts = map;
    });
  }

  type Draft = { name: string; command: string; args: string; env: string; icon: string; color: string; maestro: boolean; chat: boolean; cwd: string };
  const EMPTY: Draft = { name: "", command: "", args: "", env: "", icon: "", color: "", maestro: false, chat: false, cwd: "" };
  let draft = $state<Draft>({ ...EMPTY });
  let editing = $state<string | null>(null);

  function edit(name: string, p: AgentPreset) {
    editing = name;
    draft = {
      name,
      command: p.command,
      args: (p.args ?? []).join(" "),
      env: Object.entries(p.env ?? {})
        .map(([k, v]) => `${k}=${v}`)
        .join("\n"),
      icon: p.icon ?? "",
      color: p.color ?? "",
      maestro: !!p.maestro,
      chat: !!p.start_in_chat_mode,
      cwd: p.cwd ?? "",
    };
  }

  function parseEnv(s: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of s.split(/\n/)) {
      const i = line.indexOf("=");
      if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    return out;
  }

  async function savePreset() {
    const name = draft.name.trim();
    if (!name || !draft.command.trim()) {
      showToast("error", tr("canvas.settings.presetNeeds", "A preset needs a name and a command"));
      return;
    }
    const d = draft;
    const was = editing;
    await save((c) => {
      if (was && was !== name) delete c.agent_presets[was];
      c.agent_presets[name] = {
        command: d.command.trim(),
        args: d.args.trim() ? d.args.trim().split(/\s+/) : [],
        env: parseEnv(d.env),
        icon: d.icon.trim() || null,
        color: d.color.trim() || null,
        maestro: d.maestro,
        start_in_chat_mode: d.chat,
        cwd: d.cwd.trim() || null,
      };
    });
    editing = null;
    draft = { ...EMPTY };
  }

  function removePreset(name: string) {
    void save((c) => {
      delete c.agent_presets[name];
      if (c.default_agent === name) c.default_agent = "shell";
    });
  }

  const presetNames = $derived(Object.keys(prefs.agent_presets));
</script>

<section class="settings-section" id="settings-section-canvas">
  <h5 class="section-title">{tr("canvas.settings.title", "Canvas")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-bg">{tr("canvas.settings.background", "Background")}</label></div>
      <select id="cv-bg" class="input-text select" disabled={busy} value={prefs.background}
        onchange={(e) => save((c) => (c.background = (e.target as HTMLSelectElement).value as CanvasPrefs["background"]))}>
        <option value="grid">{tr("canvas.settings.bgGrid", "Grid")}</option>
        <option value="plain">{tr("canvas.settings.bgPlain", "Plain")}</option>
        <option value="transparent">{tr("canvas.settings.bgTransparent", "Transparent")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-sel">{tr("canvas.settings.selectionStyle", "Selection style")}</label></div>
      <select id="cv-sel" class="input-text select" disabled={busy} value={prefs.selection_style}
        onchange={(e) => save((c) => (c.selection_style = (e.target as HTMLSelectElement).value as CanvasPrefs["selection_style"]))}>
        <option value="solid">{tr("canvas.settings.selSolid", "Solid")}</option>
        <option value="dashed">{tr("canvas.settings.selDashed", "Dashed")}</option>
        <option value="corners">{tr("canvas.settings.selCorners", "Corners")}</option>
        <option value="corner_dots">{tr("canvas.settings.selCornerDots", "Corner dots")}</option>
        <option value="elevation">{tr("canvas.settings.selElevation", "Elevation")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-mat">{tr("canvas.settings.material", "Node material")}</label></div>
      <select id="cv-mat" class="input-text select" disabled={busy} value={prefs.node_material}
        onchange={(e) => save((c) => (c.node_material = (e.target as HTMLSelectElement).value as CanvasPrefs["node_material"]))}>
        <option value="opaque">{tr("canvas.settings.matOpaque", "Opaque")}</option>
        <option value="glass">{tr("canvas.settings.matGlass", "Glass")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-cord">{tr("canvas.settings.cordStyle", "Cord style")}</label></div>
      <select id="cv-cord" class="input-text select" disabled={busy} value={prefs.cord_style}
        onchange={(e) => save((c) => (c.cord_style = (e.target as HTMLSelectElement).value as CanvasPrefs["cord_style"]))}>
        <option value="bezier">{tr("canvas.settings.cordCurve", "Curve")}</option>
        <option value="rope">{tr("canvas.settings.cordRope", "Rope")}</option>
        <option value="circuit">{tr("canvas.settings.cordCircuit", "Circuit")}</option>
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-zs">{tr("canvas.settings.zoomStep", "Zoom step")}</label>
        <span class="setting-path">{tr("canvas.settings.zoomStepHint", "Per wheel notch and per ⌘+ / ⌘−")}</span>
      </div>
      <select id="cv-zs" class="input-text select" disabled={busy} value={String(prefs.zoom_step)}
        onchange={(e) => save((c) => (c.zoom_step = Number((e.target as HTMLSelectElement).value)))}>
        {#each [0.05, 0.1, 0.15, 0.2, 0.25, 0.5] as v (v)}
          <option value={String(v)}>{Math.round(v * 100)}%</option>
        {/each}
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-zm">{tr("canvas.settings.zoomModifier", "Zoom with the wheel while holding")}</label></div>
      <select id="cv-zm" class="input-text select" disabled={busy} value={prefs.zoom_modifier}
        onchange={(e) => save((c) => (c.zoom_modifier = (e.target as HTMLSelectElement).value as CanvasPrefs["zoom_modifier"]))}>
        <option value="alt">{tr("canvas.settings.modAlt", "Option / Alt")}</option>
        <option value="ctrl">{tr("canvas.settings.modCtrl", "Control")}</option>
        <option value="cmd">{tr("canvas.settings.modCmd", "Command (conflicts with the workspace swipe)")}</option>
      </select>
    </div>
    {#each [
      { k: "right_drag_pan", label: tr("canvas.settings.rightDragPan", "Pan by dragging with the right button") },
      { k: "focus_on_hover", label: tr("canvas.settings.focusOnHover", "Focus terminals, notes and portals on hover") },
      { k: "spell_check", label: tr("canvas.settings.spellCheck", "Spell check in notes, text and the composer") },
    ] as row (row.k)}
      {@const key = row.k as "right_drag_pan" | "focus_on_hover" | "spell_check"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col"><span class="setting-label">{row.label}</span></div>
        <button class="toggle" class:on={prefs[key]} disabled={busy} role="switch" aria-checked={prefs[key]} aria-label={row.label}
          onclick={() => save((c) => (c[key] = !c[key]))}><span class="toggle-knob"></span></button>
      </div>
    {/each}
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-shortcuts">
  <h5 class="section-title">{tr("canvas.settings.shortcutsTitle", "Canvas shortcuts")}</h5>
  <div class="card">
    <p class="setting-path intro">{tr("canvas.settings.shortcutsIntro", "Click a field and press the new combination. A combination already in use moves to this action. Backspace clears; Reset returns to the default.")}</p>
    {#each SHORTCUT_IDS as id, i (id)}
      {#if i > 0}<div class="divider"></div>{/if}
      <div class="setting-row">
        <div class="setting-col"><span class="setting-label">{tr(`canvas.settings.shortcut.${id}`, id)}</span></div>
        <div class="pair end">
          <button
            type="button"
            class="input-text mono shortcut"
            class:rec={recording === id}
            disabled={busy}
            onclick={() => (recording = id)}
            onblur={() => recording === id && (recording = null)}
            onkeydown={(e) => recording === id && recordKey(id, e)}
          >
            {recording === id ? tr("canvas.settings.shortcutRecording", "Press keys…") : comboLabel(currentCombo(id))}
          </button>
          {#if prefs.shortcuts && id in prefs.shortcuts}
            <button type="button" class="button" disabled={busy} onclick={() => resetShortcut(id)}>{tr("canvas.settings.shortcutReset", "Reset")}</button>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-defaults">
  <h5 class="section-title">{tr("canvas.settings.defaultsTitle", "New item defaults")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-da">{tr("canvas.settings.defaultAgent", "New terminal (⌘T) runs")}</label></div>
      <select id="cv-da" class="input-text select" disabled={busy} value={prefs.default_agent}
        onchange={(e) => save((c) => (c.default_agent = (e.target as HTMLSelectElement).value))}>
        {#each BUILTIN as a (a)}
          <option value={a}>{$t(`canvas.term.agent.${a}`, { default: a })}</option>
        {/each}
        {#each presetNames as n (n)}
          <option value={n}>{n}</option>
        {/each}
      </select>
    </div>
    {#each [
      { k: "terminal", label: tr("canvas.settings.sizeTerminal", "Terminal size") },
      { k: "note", label: tr("canvas.settings.sizeNote", "Note size") },
      { k: "text", label: tr("canvas.settings.sizeText", "Text size") },
      { k: "portal", label: tr("canvas.settings.sizePortal", "Portal size") },
    ] as row (row.k)}
      {@const key = row.k as "terminal" | "note" | "text" | "portal"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <label class="setting-label" for="cv-size-{key}">{row.label}</label>
          <span class="setting-path">{tr("canvas.settings.sizeHint", "Width × height, snapped to the 20 pt grid; empty = automatic")}</span>
        </div>
        <input id="cv-size-{key}" class="input-text" placeholder="640x400" disabled={busy} value={prefs.defaults[key].size}
          onchange={(e) => save((c) => (c.defaults[key].size = (e.target as HTMLInputElement).value.trim()))} />
      </div>
    {/each}
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-nf">{tr("canvas.settings.noteFont", "Note font")}</label></div>
      <input id="cv-nf" class="input-text" placeholder="Georgia" disabled={busy} value={prefs.defaults.note.font}
        onchange={(e) => save((c) => (c.defaults.note.font = (e.target as HTMLInputElement).value.trim()))} />
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-nr">{tr("canvas.settings.noteRender", "New notes open in")}</label></div>
      <select id="cv-nr" class="input-text select" disabled={busy} value={prefs.defaults.note.render}
        onchange={(e) => save((c) => (c.defaults.note.render = (e.target as HTMLSelectElement).value as "formatted" | "raw"))}>
        <option value="formatted">{tr("canvas.notes.formatted", "Formatted")}</option>
        <option value="raw">{tr("canvas.notes.raw", "Raw")}</option>
      </select>
    </div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-terminal">
  <h5 class="section-title">{tr("canvas.settings.terminalTitle", "Terminals")}</h5>
  <div class="card">
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-tt">{tr("canvas.settings.termTheme", "Theme")}</label></div>
      <select id="cv-tt" class="input-text select" disabled={busy} value={prefs.terminal.theme}
        onchange={(e) => save((c) => (c.terminal.theme = (e.target as HTMLSelectElement).value))}>
        {#each TERM_THEME_IDS as id (id)}
          <option value={id}>{id === "system" ? tr("canvas.settings.termThemeSystem", "Follow the app") : id}</option>
        {/each}
      </select>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-tf">{tr("canvas.settings.termFont", "Font")}</label></div>
      <input id="cv-tf" class="input-text" placeholder="JetBrains Mono" disabled={busy} value={prefs.terminal.font_family}
        onchange={(e) => save((c) => (c.terminal.font_family = (e.target as HTMLInputElement).value.trim()))} />
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-tp">{tr("canvas.settings.termPadding", "Padding (horizontal, vertical)")}</label></div>
      <div class="pair">
        <input id="cv-tp" class="input-text num" type="number" min="0" max="64" disabled={busy} value={prefs.terminal.padding.x}
          onchange={(e) => save((c) => (c.terminal.padding.x = Number((e.target as HTMLInputElement).value) || 0))} />
        <input class="input-text num" type="number" min="0" max="64" disabled={busy} value={prefs.terminal.padding.y}
          aria-label={tr("canvas.settings.termPaddingY", "Vertical padding")}
          onchange={(e) => save((c) => (c.terminal.padding.y = Number((e.target as HTMLInputElement).value) || 0))} />
      </div>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <label class="setting-label" for="cv-mem">{tr("canvas.settings.memoryLimit", "Memory limit per terminal (MB)")}</label>
        <span class="setting-path">{tr("canvas.settings.memoryLimitHint", "A program above it is stopped; the shell stays. 0 = off")}</span>
      </div>
      <input id="cv-mem" class="input-text num" type="number" min="0" step="100" disabled={busy} value={prefs.terminal.memory_limit_mb}
        onchange={(e) => save((c) => (c.terminal.memory_limit_mb = Math.max(0, Math.round(Number((e.target as HTMLInputElement).value) || 0))))} />
    </div>
    {#each [
      { k: "option_as_meta", label: tr("canvas.settings.optionAsMeta", "Option key sends Meta (instead of typing characters)") },
      { k: "shell_fallback", label: tr("canvas.settings.shellFallback", "Start another shell when the saved one is missing") },
    ] as row (row.k)}
      {@const key = row.k as "option_as_meta" | "shell_fallback"}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col"><span class="setting-label">{row.label}</span></div>
        <button class="toggle" class:on={prefs.terminal[key]} disabled={busy} role="switch" aria-checked={prefs.terminal[key]} aria-label={row.label}
          onclick={() => save((c) => (c.terminal[key] = !c.terminal[key]))}><span class="toggle-knob"></span></button>
      </div>
    {/each}
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><span class="setting-label">{tr("canvas.settings.notifyAttention", "Notify when an agent needs attention")}</span></div>
      <button class="toggle" class:on={prefs.notifications.attention} disabled={busy} role="switch" aria-checked={prefs.notifications.attention}
        aria-label={tr("canvas.settings.notifyAttention", "Notify when an agent needs attention")}
        onclick={() => save((c) => (c.notifications.attention = !c.notifications.attention))}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><span class="setting-label">{tr("canvas.settings.badgesNonMaestro", "Attention badge on terminals that are not the Maestro")}</span></div>
      <button class="toggle" class:on={prefs.notifications.badges_for_non_maestro} disabled={busy} role="switch" aria-checked={prefs.notifications.badges_for_non_maestro}
        aria-label={tr("canvas.settings.badgesNonMaestro", "Attention badge on terminals that are not the Maestro")}
        onclick={() => save((c) => (c.notifications.badges_for_non_maestro = !c.notifications.badges_for_non_maestro))}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col"><label class="setting-label" for="cv-sleep">{tr("canvas.settings.preventSleep", "Keep the computer awake")}</label></div>
      <select id="cv-sleep" class="input-text select" disabled={busy} value={prefs.power.prevent_sleep}
        onchange={(e) => save((c) => (c.power.prevent_sleep = (e.target as HTMLSelectElement).value as CanvasPrefs["power"]["prevent_sleep"]))}>
        <option value="while_working">{tr("canvas.settings.sleepWorking", "While agents are working")}</option>
        <option value="always">{tr("canvas.settings.sleepAlways", "Always")}</option>
        <option value="never">{tr("canvas.settings.sleepNever", "Never")}</option>
      </select>
    </div>
  </div>
</section>

<section class="settings-section" id="settings-section-canvas-presets">
  <h5 class="section-title">{tr("canvas.settings.presetsTitle", "Agent presets")}</h5>
  <div class="card">
    <p class="setting-path intro">{tr("canvas.settings.presetsHint", "Bring your own agent: any command line, with its arguments and environment. It shows up in Add → Terminal and can be the default for ⌘T. Editing a preset changes only terminals made afterwards.")}</p>
    {#each presetNames as name (name)}
      {@const p = prefs.agent_presets[name]}
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">
            {#if p.color}<span class="swatch" style:background={p.color}></span>{/if}{name}
          </span>
          <span class="setting-path mono">{[p.command, ...(p.args ?? [])].join(" ")}</span>
        </div>
        <div class="pair">
          <button class="button" disabled={busy} onclick={() => edit(name, p)}>{tr("canvas.settings.edit", "Edit")}</button>
          <button class="button danger" disabled={busy} onclick={() => removePreset(name)}>{tr("canvas.settings.remove", "Remove")}</button>
        </div>
      </div>
      <div class="divider"></div>
    {/each}
    <div class="editor">
      <label>{tr("canvas.settings.presetName", "Name")}<input class="input-text" bind:value={draft.name} placeholder="Aider" /></label>
      <label>{tr("canvas.settings.presetCommand", "Command")}<input class="input-text mono" bind:value={draft.command} placeholder="aider" spellcheck="false" /></label>
      <label>{tr("canvas.settings.presetArgs", "Arguments")}<input class="input-text mono" bind:value={draft.args} placeholder="--model sonnet" spellcheck="false" /></label>
      <label>{tr("canvas.settings.presetEnv", "Environment (one KEY=value per line)")}<textarea class="input-text mono" rows="2" bind:value={draft.env} spellcheck="false"></textarea></label>
      <label>{tr("canvas.settings.presetCwd", "Folder (empty = the workspace's)")}<input class="input-text mono" bind:value={draft.cwd} spellcheck="false" /></label>
      <div class="pair">
        <label>{tr("canvas.settings.presetIcon", "Icon")}<input class="input-text" bind:value={draft.icon} placeholder="hammer" /></label>
        <label>{tr("canvas.settings.presetColor", "Color")}<input class="input-text" bind:value={draft.color} placeholder="#FF8800" /></label>
      </div>
      <label class="check"><input type="checkbox" bind:checked={draft.maestro} />{tr("canvas.settings.presetMaestro", "Maestro (can recruit and manage other agents)")}</label>
      <label class="check"><input type="checkbox" bind:checked={draft.chat} />{tr("canvas.settings.presetChat", "Start in chat mode")}</label>
      <div class="pair end">
        {#if editing}
          <button class="button" disabled={busy} onclick={() => { editing = null; draft = { ...EMPTY }; }}>{tr("canvas.settings.cancel", "Cancel")}</button>
        {/if}
        <button class="button primary" disabled={busy} onclick={savePreset}>
          {editing ? tr("canvas.settings.savePreset", "Save preset") : tr("canvas.settings.addPreset", "Add preset")}
        </button>
      </div>
    </div>
  </div>
</section>

<SettingsCanvasImages />

<SettingsCanvasEnvironments />

<style>
  .pair {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pair.end {
    justify-content: flex-end;
  }
  .num {
    width: 88px;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
  .intro {
    margin: 4px 0 10px;
  }
  .swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    margin-right: 6px;
    vertical-align: middle;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 8px;
  }
  .editor label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted);
    flex: 1;
  }
  .editor label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  .danger {
    color: var(--error, #ff453a);
  }
  .shortcut {
    min-width: 120px;
    text-align: center;
    cursor: pointer;
  }
  .shortcut.rec {
    outline: 2px solid var(--accent);
  }
</style>
