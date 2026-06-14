<script lang="ts">
  import { t } from "$lib/i18n";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import { getSettings, updateSettings, toggleBool } from "../settings-helpers";

  let { embedded = false }: { embedded?: boolean } = $props();

  let settings = $derived(getSettings());

  let hotkeyInput = $state("");
  let hotkeyTimer: ReturnType<typeof setTimeout> | null = null;
  let hotkeyMode = $state<"record" | "type">("record");
  let hotkeyRecording = $state(false);
  let musicHotkeyInput = $state("");
  let musicHotkeyTimer: ReturnType<typeof setTimeout> | null = null;
  let musicHotkeyMode = $state<"record" | "type">("record");
  let musicHotkeyRecording = $state(false);

  $effect(() => {
    if (settings) {
      hotkeyInput = settings.download.hotkey_binding;
      musicHotkeyInput = settings.download.music_hotkey_binding;
    }
  });

  function handleHotkeyInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    hotkeyInput = value;
    if (hotkeyTimer) clearTimeout(hotkeyTimer);
    hotkeyTimer = setTimeout(async () => {
      if (value.trim()) {
        await updateSettings({ download: { hotkey_binding: value } });
      }
    }, 800);
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

  function handleHotkeyKeyDown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
    const keyName = mapKeyName(e.key);
    if (!keyName) return;
    const parts: string[] = [];
    if (e.ctrlKey || e.metaKey) parts.push("CmdOrCtrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    parts.push(keyName);
    const value = parts.join("+");
    hotkeyInput = value;
    hotkeyRecording = false;
    updateSettings({ download: { hotkey_binding: value } });
  }

  function handleMusicHotkeyInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    musicHotkeyInput = value;
    if (musicHotkeyTimer) clearTimeout(musicHotkeyTimer);
    musicHotkeyTimer = setTimeout(async () => {
      if (value.trim()) {
        await updateSettings({ download: { music_hotkey_binding: value } });
      }
    }, 800);
  }

  function handleMusicHotkeyKeyDown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
    const keyName = mapKeyName(e.key);
    if (!keyName) return;
    const parts: string[] = [];
    if (e.ctrlKey || e.metaKey) parts.push("CmdOrCtrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    parts.push(keyName);
    const value = parts.join("+");
    musicHotkeyInput = value;
    musicHotkeyRecording = false;
    updateSettings({ download: { music_hotkey_binding: value } });
  }

  function changeMusicAudioFormat(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { music_audio_format: value } });
  }
</script>

{#if settings}
  {#if !embedded}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.download.section_clipboard_hotkeys')}</h5>
      <p class="settings-section-hint">{$t('settings.download.section_clipboard_hotkeys_desc')}</p>
    </div>
  {/if}

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.clipboard_detection')} <ContextHint text={$t('hints.clipboard') as string} dismissKey="clipboard" /></span>
        <span class="setting-path">{$t('settings.download.clipboard_detection_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.clipboard_detection} onclick={() => toggleBool("download", "clipboard_detection", settings.download.clipboard_detection)} role="switch" aria-checked={settings.download.clipboard_detection} aria-label={$t('settings.download.clipboard_detection') as string}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.auto_download_on_paste')}</span>
        <span class="setting-path">{$t('settings.download.auto_download_on_paste_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.auto_download_on_paste} onclick={() => toggleBool("download", "auto_download_on_paste", settings.download.auto_download_on_paste)} role="switch" aria-checked={settings.download.auto_download_on_paste} aria-label={$t('settings.download.auto_download_on_paste') as string}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.hotkey_enabled')} <ContextHint text={$t('hints.hotkey') as string} dismissKey="hotkey" /></span>
        <span class="setting-path">{$t('settings.download.hotkey_enabled_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.hotkey_enabled} onclick={() => toggleBool("download", "hotkey_enabled", settings.download.hotkey_enabled)} role="switch" aria-checked={settings.download.hotkey_enabled} aria-label={$t('settings.download.hotkey_enabled') as string}><span class="toggle-knob"></span></button>
    </div>
    {#if settings.download.hotkey_enabled}
      <div class="divider"></div>
      <div class="setting-row hotkey-row">
        <span class="setting-label">{$t('settings.download.hotkey_binding')}</span>
        <div class="hotkey-controls">
          <div class="hotkey-mode-switch">
            <button class="hotkey-mode-btn" class:active={hotkeyMode === 'record'} onclick={() => { hotkeyMode = 'record'; hotkeyRecording = false; }}>{$t('settings.download.hotkey_record')}</button>
            <button class="hotkey-mode-btn" class:active={hotkeyMode === 'type'} onclick={() => { hotkeyMode = 'type'; hotkeyRecording = false; }}>{$t('settings.download.hotkey_type')}</button>
          </div>
          {#if hotkeyMode === 'type'}
            <input type="text" class="input-hotkey" value={hotkeyInput} oninput={handleHotkeyInput} spellcheck="false" />
          {:else}
            <button class="input-hotkey hotkey-record-btn" class:recording={hotkeyRecording} onclick={() => { hotkeyRecording = true; }} onkeydown={hotkeyRecording ? handleHotkeyKeyDown : undefined} onblur={() => { hotkeyRecording = false; }}>
              {hotkeyRecording ? $t('settings.download.hotkey_press') : (hotkeyInput || $t('settings.download.hotkey_press'))}
            </button>
          {/if}
        </div>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.copy_to_clipboard_on_hotkey')}</span>
          <span class="setting-path">{$t('settings.download.copy_to_clipboard_on_hotkey_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.copy_to_clipboard_on_hotkey} onclick={() => toggleBool("download", "copy_to_clipboard_on_hotkey", settings.download.copy_to_clipboard_on_hotkey)} role="switch" aria-checked={settings.download.copy_to_clipboard_on_hotkey} aria-label={$t('settings.download.copy_to_clipboard_on_hotkey') as string}><span class="toggle-knob"></span></button>
      </div>
    {/if}
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.music_hotkey_enabled')}</span>
        <span class="setting-path">{$t('settings.download.music_hotkey_enabled_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.music_hotkey_enabled} onclick={() => toggleBool("download", "music_hotkey_enabled", settings.download.music_hotkey_enabled)} role="switch" aria-checked={settings.download.music_hotkey_enabled} aria-label={$t('settings.download.music_hotkey_enabled') as string}><span class="toggle-knob"></span></button>
    </div>
    {#if settings.download.music_hotkey_enabled}
      <div class="divider"></div>
      <div class="setting-row hotkey-row">
        <span class="setting-label">{$t('settings.download.music_hotkey_binding')}</span>
        <div class="hotkey-controls">
          <div class="hotkey-mode-switch">
            <button class="hotkey-mode-btn" class:active={musicHotkeyMode === 'record'} onclick={() => { musicHotkeyMode = 'record'; musicHotkeyRecording = false; }}>{$t('settings.download.hotkey_record')}</button>
            <button class="hotkey-mode-btn" class:active={musicHotkeyMode === 'type'} onclick={() => { musicHotkeyMode = 'type'; musicHotkeyRecording = false; }}>{$t('settings.download.hotkey_type')}</button>
          </div>
          {#if musicHotkeyMode === 'type'}
            <input type="text" class="input-hotkey" value={musicHotkeyInput} oninput={handleMusicHotkeyInput} spellcheck="false" />
          {:else}
            <button class="input-hotkey hotkey-record-btn" class:recording={musicHotkeyRecording} onclick={() => { musicHotkeyRecording = true; }} onkeydown={musicHotkeyRecording ? handleMusicHotkeyKeyDown : undefined} onblur={() => { musicHotkeyRecording = false; }}>
              {musicHotkeyRecording ? $t('settings.download.hotkey_press') : (musicHotkeyInput || $t('settings.download.hotkey_press'))}
            </button>
          {/if}
        </div>
      </div>
    {/if}
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.music_audio_format')}</span>
        <span class="setting-path">{$t('settings.download.music_audio_format_desc')}</span>
      </div>
      <select class="select" value={settings.download.music_audio_format} onchange={changeMusicAudioFormat}>
        <option value="m4a">M4A (AAC)</option>
        <option value="mp3">MP3</option>
        <option value="flac">FLAC (lossless)</option>
        <option value="opus">Opus</option>
        <option value="wav">WAV</option>
      </select>
    </div>
  </div>
{/if}
