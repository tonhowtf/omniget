<script lang="ts">
  import { t } from "$lib/i18n";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import { getSettings, toggleBool } from "../settings-helpers";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingsSaveBar from "../rebrand/SettingsSaveBar.svelte";
  import MoreOptions from "./MoreOptions.svelte";
  import Toggle from "./Toggle.svelte";
  import KeyAnchor from "./KeyAnchor.svelte";

  type ControlArgs = { id: string; describedBy: string | undefined };
  type BindingField = "hotkey_binding" | "music_hotkey_binding";

  const HOTKEY_CHILD_KEYS = ["settings.download.hotkey_binding", "settings.download.copy_to_clipboard_on_hotkey"];
  const MUSIC_CHILD_KEYS = ["settings.download.music_hotkey_binding"];

  let settings = $derived(getSettings());

  const hotkeys = createSettingsDraft({
    id: "hotkeys",
    read: (s) => ({
      hotkey_binding: s.download.hotkey_binding,
      music_hotkey_binding: s.download.music_hotkey_binding,
    }),
    toPatch: (d) => ({
      download: {
        hotkey_binding: d.hotkey_binding.trim(),
        music_hotkey_binding: d.music_hotkey_binding.trim(),
      },
    }),
  });

  let modes = $state<Record<BindingField, "record" | "type">>({ hotkey_binding: "record", music_hotkey_binding: "record" });
  let recording = $state<BindingField | null>(null);

  let bindingMissing = $derived(
    !!settings &&
      ((settings.download.hotkey_enabled && !(hotkeys.value.hotkey_binding ?? "").trim()) ||
        (settings.download.music_hotkey_enabled && !(hotkeys.value.music_hotkey_binding ?? "").trim())),
  );

  let moreOpen = $state(false);

  $effect(() => {
    if (hotkeys.dirty || hotkeys.error) moreOpen = true;
  });

  function saveHotkeys() {
    if (bindingMissing) return;
    void hotkeys.save();
  }

  function mapKeyName(key: string): string | null {
    if (key.length === 1 && /[a-zA-Z]/.test(key)) return key.toUpperCase();
    if (key.length === 1 && /[0-9]/.test(key)) return key;
    if (/^F([1-9]|1[0-2])$/.test(key)) return key;
    const map: Record<string, string> = {
      " ": "Space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
      Enter: "Enter", Tab: "Tab", Escape: "Escape", Backspace: "Backspace", Delete: "Delete",
      Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Insert: "Insert",
    };
    return map[key] ?? null;
  }

  function startRecording(field: BindingField, e: MouseEvent) {
    recording = field;
    (e.currentTarget as HTMLButtonElement | null)?.focus();
  }

  function handleRecordKeyDown(field: BindingField, e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = null;
      return;
    }
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
    const keyName = mapKeyName(e.key);
    if (!keyName) return;
    const parts: string[] = [];
    if (e.ctrlKey || e.metaKey) parts.push("CmdOrCtrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    parts.push(keyName);
    hotkeys.value[field] = parts.join("+");
    recording = null;
  }

  function setMode(field: BindingField, mode: "record" | "type") {
    modes = { ...modes, [field]: mode };
    recording = null;
  }
</script>

{#snippet binding(c: ControlArgs, field: BindingField)}
  <div class="hotkey-controls">
    <div class="hotkey-mode-switch" role="group" aria-label={$t("settings.download.hotkey_binding")}>
      <button type="button" class="hotkey-mode-btn" class:active={modes[field] === "record"} aria-pressed={modes[field] === "record"} onclick={() => setMode(field, "record")}>{$t("settings.download.hotkey_record")}</button>
      <button type="button" class="hotkey-mode-btn" class:active={modes[field] === "type"} aria-pressed={modes[field] === "type"} onclick={() => setMode(field, "type")}>{$t("settings.download.hotkey_type")}</button>
    </div>
    {#if modes[field] === "type"}
      <input id={c.id} aria-describedby={c.describedBy} type="text" class="input-hotkey" dir="ltr" bind:value={hotkeys.value[field]} spellcheck="false" />
    {:else}
      <button
        id={c.id}
        aria-describedby={c.describedBy}
        type="button"
        class="input-hotkey hotkey-record-btn"
        class:recording={recording === field}
        onclick={(e) => startRecording(field, e)}
        onkeydown={recording === field ? (e) => handleRecordKeyDown(field, e) : undefined}
        onblur={() => { if (recording === field) recording = null; }}
      >
        {recording === field ? $t("settings.download.hotkey_press") : (hotkeys.value[field] || $t("settings.download.hotkey_press"))}
      </button>
    {/if}
  </div>
{/snippet}

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="shortcuts" icon="keyboard" title={$t("settings.downloads.shortcuts.title")}>
    <SettingRow id="dl-clipboard-detection" label={$t("settings.download.clipboard_detection")} help={$t("settings.download.clipboard_detection_desc")} settingKey="settings.download.clipboard_detection" status="download.clipboard_detection">
      {#snippet control(c)}
        <span class="hint-slot"><ContextHint text={$t("hints.clipboard") as string} dismissKey="clipboard" /></span>
        <Toggle id={c.id} describedBy={c.describedBy} on={d.clipboard_detection} onclick={() => toggleBool("download", "clipboard_detection", d.clipboard_detection)} />
      {/snippet}
    </SettingRow>
    <SettingRow id="dl-auto-download-on-paste" label={$t("settings.download.auto_download_on_paste")} help={$t("settings.download.auto_download_on_paste_desc")} settingKey="settings.download.auto_download_on_paste" status="download.auto_download_on_paste">
      {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.auto_download_on_paste} onclick={() => toggleBool("download", "auto_download_on_paste", d.auto_download_on_paste)} />{/snippet}
    </SettingRow>

    <MoreOptions id="shortcuts" bind:open={moreOpen}>
      <SettingRow id="dl-hotkey-enabled" label={$t("settings.download.hotkey_enabled")} help={$t("settings.download.hotkey_enabled_desc")} settingKey="settings.download.hotkey_enabled" status="download.hotkey_enabled">
        {#snippet control(c)}
          <span class="hint-slot"><ContextHint text={$t("hints.hotkey") as string} dismissKey="hotkey" /></span>
          <KeyAnchor keys={HOTKEY_CHILD_KEYS} active={!d.hotkey_enabled}><Toggle id={c.id} describedBy={c.describedBy} on={d.hotkey_enabled} onclick={() => toggleBool("download", "hotkey_enabled", d.hotkey_enabled)} /></KeyAnchor>
        {/snippet}
      </SettingRow>
      {#if d.hotkey_enabled}
        <SettingRow id="dl-hotkey-binding" label={$t("settings.download.hotkey_binding")} settingKey="settings.download.hotkey_binding" status="download.hotkey_binding">
          {#snippet control(c)}{@render binding(c, "hotkey_binding")}{/snippet}
        </SettingRow>
        <SettingRow id="dl-copy-to-clipboard-on-hotkey" label={$t("settings.download.copy_to_clipboard_on_hotkey")} help={$t("settings.download.copy_to_clipboard_on_hotkey_desc")} settingKey="settings.download.copy_to_clipboard_on_hotkey" status="download.copy_to_clipboard_on_hotkey">
          {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.copy_to_clipboard_on_hotkey} onclick={() => toggleBool("download", "copy_to_clipboard_on_hotkey", d.copy_to_clipboard_on_hotkey)} />{/snippet}
        </SettingRow>
      {/if}
      <SettingRow id="dl-music-hotkey-enabled" label={$t("settings.downloads.shortcuts.music_hotkey")} help={$t("settings.downloads.shortcuts.music_hotkey_desc")} settingKey="settings.downloads.shortcuts.music_hotkey" status="download.music_hotkey_enabled">
        {#snippet control(c)}<KeyAnchor keys={MUSIC_CHILD_KEYS} active={!d.music_hotkey_enabled}><Toggle id={c.id} describedBy={c.describedBy} on={d.music_hotkey_enabled} onclick={() => toggleBool("download", "music_hotkey_enabled", d.music_hotkey_enabled)} /></KeyAnchor>{/snippet}
      </SettingRow>
      {#if d.music_hotkey_enabled}
        <SettingRow id="dl-music-hotkey-binding" label={$t("settings.download.music_hotkey_binding")} settingKey="settings.download.music_hotkey_binding" status="download.music_hotkey_binding">
          {#snippet control(c)}{@render binding(c, "music_hotkey_binding")}{/snippet}
        </SettingRow>
      {/if}
      {#if bindingMissing}
        <p class="binding-error" role="alert">{$t("settings.downloads.shortcuts.binding_missing")}</p>
      {/if}
      <SettingsSaveBar dirty={hotkeys.dirty} saving={hotkeys.saving} error={hotkeys.error} onSave={saveHotkeys} onDiscard={() => { recording = null; hotkeys.discard(); }} />
    </MoreOptions>
  </SettingGroup>
{/if}

<style>
  .binding-error { margin: var(--space-1) 0 0; color: var(--error); font-size: var(--text-base); line-height: 1.5; }
  .hint-slot { display: inline-flex; margin-inline-end: var(--space-2); }
  .hotkey-controls { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; }
</style>
