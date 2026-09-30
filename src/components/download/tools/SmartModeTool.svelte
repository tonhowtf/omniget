<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";
  import { pluralKey } from "$lib/home/plural";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    HEIGHTS,
    downloadArgs,
    linksIn,
    loadState,
    saveState,
    validSubs,
    type SmartPreset,
    type SmartState,
  } from "$lib/download-tools/smart-presets";

  let smart = $state<SmartState>(loadState());
  let text = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let editing = $state<SmartPreset | null>(null);

  const active = $derived(smart.presets.find((p) => p.id === smart.activeId) ?? smart.presets[0]);
  const links = $derived(linksIn(text));

  function persist() {
    saveState($state.snapshot(smart));
  }

  function nameOf(p: SmartPreset): string {
    if (p.name.trim()) return p.name.trim();
    const key = `downloads.tools.smart_mode.preset_${p.id.replace(/-/g, "_")}`;
    const label = $t(key);
    return label === key ? $t("downloads.tools.smart_mode.untitled") : label;
  }

  function summary(p: SmartPreset): string {
    const parts: string[] = [];
    if (p.mode === "audio") parts.push($t("downloads.tools.smart_mode.audio"));
    else parts.push(p.height ? `${p.height}p` : $t("downloads.tools.smart_mode.best"));
    if (p.mode === "video" && p.subs.trim()) parts.push($t("downloads.tools.smart_mode.subs_summary", { langs: p.subs.trim() }));
    parts.push(p.folder ? p.folder.split(/[\\/]/).filter(Boolean).pop() ?? p.folder : $t("downloads.tools.smart_mode.default_folder"));
    return parts.join(" · ");
  }

  function choose(id: string) {
    smart.activeId = id;
    persist();
  }

  async function run(urls: string[]) {
    if (!active || busy || urls.length === 0) return;
    const fallback = getSettings()?.download.default_output_dir ?? "";
    if (!active.folder && !fallback) {
      error = $t("downloads.tools.smart_mode.no_folder");
      return;
    }
    busy = true;
    error = null;
    let queued = 0;
    const failed: string[] = [];
    for (const url of urls) {
      try {
        await invoke("download_from_url", downloadArgs(active, url, fallback));
        queued++;
      } catch (e: any) {
        const msg = typeof e === "string" ? e : (e?.message ?? "");
        failed.push(msg ? translateBackendError(msg, $t) : $t("common.error"));
      }
    }
    busy = false;
    if (queued > 0) {
      showToast("success", $t(pluralKey("downloads.tools.smart_mode.queued", queued), { count: queued, preset: nameOf(active) }));
      text = "";
    }
    if (failed.length > 0) error = failed[0];
  }

  async function pasteAndGo() {
    let clip = "";
    try {
      clip = (await readText()) ?? "";
    } catch {
      clip = "";
    }
    const found = linksIn(clip);
    if (found.length === 0) {
      error = $t("downloads.tools.smart_mode.no_link");
      return;
    }
    await run(found);
  }

  function startNew() {
    editing = { id: `p${Date.now()}`, name: "", mode: "video", height: 1080, subs: "", folder: "" };
  }

  function startEdit(p: SmartPreset) {
    editing = { ...p };
  }

  async function pickFolder() {
    if (!editing) return;
    const picked = await open({ directory: true, title: $t("downloads.tools.smart_mode.folder") });
    if (typeof picked === "string") editing.folder = picked;
  }

  function saveEdit() {
    if (!editing || !validSubs(editing.subs)) return;
    const copy = { ...editing, name: editing.name.trim(), subs: editing.subs.trim() };
    const i = smart.presets.findIndex((p) => p.id === copy.id);
    if (i >= 0) smart.presets[i] = copy;
    else smart.presets.push(copy);
    smart.activeId = copy.id;
    editing = null;
    persist();
  }

  function remove(p: SmartPreset) {
    if (smart.presets.length <= 1) return;
    smart.presets = smart.presets.filter((x) => x.id !== p.id);
    if (smart.activeId === p.id) smart.activeId = smart.presets[0].id;
    if (editing?.id === p.id) editing = null;
    persist();
  }
</script>

<div class="sm" data-agent-readiness={busy ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.smart_mode.lead")}</p>

  <div class="presets" role="radiogroup" aria-label={$t("downloads.tools.smart_mode.presets")}>
    {#each smart.presets as p (p.id)}
      <div class="preset" class:on={p.id === active?.id}>
        <button
          class="pick"
          role="radio"
          aria-checked={p.id === active?.id}
          data-preset={p.id}
          onclick={() => choose(p.id)}
        >
          <span class="name">{nameOf(p)}</span>
          <span class="dim small">{summary(p)}</span>
        </button>
        <div class="acts">
          <button class="link" onclick={() => startEdit(p)}>{$t("downloads.tools.smart_mode.edit")}</button>
          {#if smart.presets.length > 1}
            <button class="link" onclick={() => remove(p)}>{$t("downloads.tools.smart_mode.remove")}</button>
          {/if}
        </div>
      </div>
    {/each}
    <button class="preset add" onclick={startNew}>+ {$t("downloads.tools.smart_mode.new")}</button>
  </div>

  {#if editing}
    <form
      class="editor"
      onsubmit={(e) => {
        e.preventDefault();
        saveEdit();
      }}
    >
      <label>
        <span>{$t("downloads.tools.smart_mode.name")}</span>
        <input bind:value={editing.name} placeholder={nameOf(editing)} maxlength="40" />
      </label>
      <label>
        <span>{$t("downloads.tools.smart_mode.kind")}</span>
        <select bind:value={editing.mode}>
          <option value="video">{$t("downloads.tools.smart_mode.video")}</option>
          <option value="audio">{$t("downloads.tools.smart_mode.audio")}</option>
        </select>
      </label>
      {#if editing.mode === "video"}
        <label>
          <span>{$t("downloads.tools.smart_mode.quality")}</span>
          <select bind:value={editing.height}>
            {#each HEIGHTS as h (h)}
              <option value={h}>{h ? `${h}p` : $t("downloads.tools.smart_mode.best")}</option>
            {/each}
          </select>
        </label>
        <label>
          <span>{$t("downloads.tools.smart_mode.subs")}</span>
          <input
            bind:value={editing.subs}
            placeholder={$t("downloads.tools.smart_mode.subs_placeholder")}
            aria-invalid={!validSubs(editing.subs)}
            spellcheck="false"
            autocomplete="off"
          />
        </label>
      {/if}
      <div class="folder-row">
        <span>{$t("downloads.tools.smart_mode.folder")}</span>
        <span class="dim small path" title={editing.folder}>{editing.folder || $t("downloads.tools.smart_mode.default_folder")}</span>
        <button type="button" class="link" onclick={pickFolder}>{$t("downloads.tools.smart_mode.choose")}</button>
        {#if editing.folder}
          <button type="button" class="link" onclick={() => editing && (editing.folder = "")}>{$t("downloads.tools.smart_mode.use_default")}</button>
        {/if}
      </div>
      {#if !validSubs(editing.subs)}
        <p class="error small">{$t("downloads.tools.smart_mode.subs_invalid")}</p>
      {/if}
      <div class="buttons">
        <button class="button" type="submit" disabled={!validSubs(editing.subs)}>{$t("downloads.tools.smart_mode.save")}</button>
        <button class="link" type="button" onclick={() => (editing = null)}>{$t("downloads.tools.smart_mode.cancel")}</button>
      </div>
    </form>
  {/if}

  <div class="go">
    <button class="button" onclick={pasteAndGo} disabled={busy || !active}>
      {$t("downloads.tools.smart_mode.paste_go", { preset: active ? nameOf(active) : "" })}
    </button>
    <textarea
      bind:value={text}
      rows="3"
      placeholder={$t("downloads.tools.smart_mode.placeholder")}
      aria-label={$t("downloads.tools.smart_mode.placeholder")}
      spellcheck="false"
    ></textarea>
    <button class="button" onclick={() => run(links)} disabled={busy || links.length === 0}>
      {busy ? $t("downloads.tools.smart_mode.working") : $t(pluralKey("downloads.tools.smart_mode.download", links.length), { count: links.length })}
    </button>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .sm {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .presets {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-2);
  }
  .preset {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .preset.on {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .preset.add {
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
    border: none;
    cursor: pointer;
  }
  .pick {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 0;
    border: none;
    background: none;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .name {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .acts {
    display: flex;
    gap: var(--space-2);
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-xs, 11px);
    cursor: pointer;
  }
  .editor {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .editor label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-xs, 11px);
    color: var(--text-dim);
  }
  .editor input,
  .editor select,
  .go textarea {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--bg, var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .editor input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 var(--hairline) var(--danger, #d33);
  }
  .folder-row,
  .buttons {
    grid-column: 1 / -1;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-xs, 11px);
    color: var(--text-dim);
    min-width: 0;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .go {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }
  .go textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    font-family: inherit;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
</style>
