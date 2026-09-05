<script lang="ts">
  /** Ditado: atalho global grava o microfone, o whisper local transcreve e o texto é digitado onde o cursor estiver. */
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type Device = { id: string; name: string };
  type Opts = { model: string; language: string; device: string; output: "type" | "paste" | "clipboard"; trailing_space: boolean };
  type State = { phase: string; seconds: number; last_text: string; error: string | null };
  type Model = { id: string; installed: boolean; label?: string; name?: string };
  type WhisperStatus = { installed: boolean; models: Model[] };

  let devices = $state<Device[]>([]);
  let opts = $state<Opts>({ model: "base", language: "auto", device: "", output: "type", trailing_space: true });
  let st = $state<State>({ phase: "idle", seconds: 0, last_text: "", error: null });
  let whisper = $state<WhisperStatus | null>(null);
  let hotkey = $state("");
  let hotkeyDraft = $state("");
  let timer: ReturnType<typeof setInterval> | null = null;
  let unlisten: (() => void) | null = null;

  async function poll() { try { st = await invoke<State>("tool_dictation_state"); } catch { /* ignore */ } }
  onMount(async () => {
    try {
      opts = await invoke<Opts>("tool_dictation_options");
      const map = await invoke<Record<string, string>>("tool_hotkeys_get"); hotkey = map.dictation ?? ""; hotkeyDraft = hotkey;
      whisper = await invoke<WhisperStatus>("tool_whisper_status");
      devices = await invoke<Device[]>("tool_dictation_devices");
    } catch (e) { showToast("error", errText(e)); }
    await poll(); timer = setInterval(poll, 500);
    unlisten = await listen<{ phase: string; error?: string; warning?: string; text?: string }>("tool-dictation", (e) => {
      if (e.payload.error) showToast("error", e.payload.error);
      if (e.payload.warning) showToast("info", e.payload.warning);
      poll();
    });
  });
  onDestroy(() => { if (timer) clearInterval(timer); unlisten?.(); });

  async function saveOpts() { try { opts = await invoke<Opts>("tool_dictation_set_options", { opts }); } catch (e) { showToast("error", errText(e)); } }
  async function start() { await saveOpts(); try { st = await invoke<State>("tool_dictation_start"); } catch (e) { showToast("error", errText(e)); } }
  async function stop() { try { const r = await invoke<{ text: string; delivered: string }>("tool_dictation_stop"); showToast("success", r.text.slice(0, 80)); await poll(); } catch (e) { showToast("error", errText(e)); await poll(); } }
  async function saveHotkey() {
    try { const map = await invoke<Record<string, string>>("tool_hotkey_set", { action: "dictation", binding: hotkeyDraft }); hotkey = map.dictation ?? ""; showToast("success", $t("tools.common.done") as string); }
    catch (e) { showToast("error", errText(e)); }
  }
  let installedModels = $derived((whisper?.models ?? []).filter((m) => m.installed));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t(`tools.dictation.phase_${st.phase}`)}{#if st.phase === "recording"} · {st.seconds}s{/if}</div>
          <div class="group-row-sub">{st.last_text ? `“${st.last_text.slice(0, 140)}”` : $t("tools.dictation.hint")}{#if st.error} · <span class="err">{st.error}</span>{/if}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if st.phase === "recording"}<button class="btn btn-danger" type="button" onclick={stop}>{$t("tools.dictation.stop")}</button>
          {:else}<button class="btn btn-primary" type="button" disabled={st.phase === "transcribing" || !whisper?.installed || !installedModels.length} onclick={start}>{$t("tools.dictation.start")}</button>{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.hotkey")}</div><div class="group-row-sub">{hotkey ? `${hotkey} · ${$t("tools.dictation.hotkey_hint")}` : $t("tools.autoclick.hotkey_none")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input mono" type="text" placeholder="CmdOrCtrl+Shift+D" bind:value={hotkeyDraft} style:width="12em" /><button class="btn btn-secondary btn-sm" type="button" onclick={saveHotkey}>{$t("tools.common.save")}</button></div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">Whisper</div><div class="group-row-sub">{#if !whisper}…{:else if !whisper.installed}{$t("tools.dictation.whisper_missing")}{:else if !installedModels.length}{$t("tools.dictation.model_missing")}{:else}{installedModels.length} {$t("tools.dictation.models")}{/if}</div></div>
        <div class="group-row-trailing btn-row">
          <a class="btn btn-ghost btn-sm" href="/tools/speech/speech-transcribe">{$t("tools.dictation.open_whisper")}</a>
          <select class="input" bind:value={opts.model} onchange={saveOpts}>{#each installedModels as m (m.id)}<option value={m.id}>{m.label ?? m.name ?? m.id}</option>{/each}</select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dictation.language")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={opts.language} onchange={saveOpts} style:width="6em" placeholder="auto" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dictation.device")}</div></div>
        <div class="group-row-trailing"><select class="input" bind:value={opts.device} onchange={saveOpts}><option value="">{$t("tools.dictation.device_default")}</option>{#each devices as d (d.id)}<option value={d.id}>{d.name}</option>{/each}</select></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dictation.output")}</div><div class="group-row-sub">{$t(`tools.dictation.output_${opts.output}_hint`)}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={opts.output} onchange={saveOpts}><option value="type">{$t("tools.dictation.output_type")}</option><option value="paste">{$t("tools.dictation.output_paste")}</option><option value="clipboard">{$t("tools.dictation.output_clipboard")}</option></select>
          <label class="check"><input type="checkbox" bind:checked={opts.trailing_space} onchange={saveOpts} /> {$t("tools.dictation.space")}</label>
        </div>
      </div>
      <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.dictation.perm")}</div></div></div>
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .err { color: var(--danger); }
  .check { display: inline-flex; align-items: center; gap: 6px; font-size: var(--text-sm); }
  .check input { accent-color: var(--accent); }
</style>
