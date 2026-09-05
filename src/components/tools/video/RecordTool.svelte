<script lang="ts">
  /** Gravar tela (estudo 28, ShareX): FFmpeg com captura nativa, microfone e buffer de replay. */
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, pickDir, reveal } from "$lib/tools/rt";

  type Source = { id: string; name: string; kind: "screen" | "audio" };
  type State = { running: boolean; replay: boolean; seconds: number; output: string | null; last_saved: string | null; error: string | null };
  let sources = $state<Source[]>([]);
  let screen = $state("");
  let audio = $state("");
  let fps = $state(30);
  let crf = $state(23);
  let cursor = $state(true);
  let replay = $state(false);
  let replaySeconds = $state(30);
  let outDir = $state("");
  let useArea = $state(false);
  let area = $state({ x: 0, y: 0, w: 1280, h: 720 });
  let st = $state<State>({ running: false, replay: false, seconds: 0, output: null, last_saved: null, error: null });
  let busy = $state(false);
  let hotkey = $state("");
  let hotkeyDraft = $state("");
  let timer: ReturnType<typeof setInterval> | null = null;
  let unlisten: (() => void) | null = null;

  async function poll() { try { st = await invoke<State>("tool_record_state"); } catch { /* ignore */ } }
  onMount(async () => {
    try {
      sources = await invoke<Source[]>("tool_record_sources");
      screen = sources.find((s) => s.kind === "screen")?.id ?? "";
      const map = await invoke<Record<string, string>>("tool_hotkeys_get"); hotkey = map.record ?? ""; hotkeyDraft = hotkey;
    } catch (e) { showToast("error", errText(e)); }
    await poll(); timer = setInterval(poll, 1000);
    unlisten = await listen<{ saved?: string; error?: string }>("tool-record", (e) => { if (e.payload.error) showToast("error", e.payload.error); if (e.payload.saved) showToast("success", e.payload.saved); poll(); });
  });
  onDestroy(() => { if (timer) clearInterval(timer); unlisten?.(); });

  async function start() {
    busy = true;
    try {
      st = await invoke<State>("tool_record_start", { opts: { screen, fps, audio, output_dir: outDir, replay_seconds: replay ? replaySeconds : 0, area: useArea ? [area.x, area.y, area.w, area.h] : null, crf, cursor } });
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
  async function stop() { busy = true; try { st = await invoke<State>("tool_record_stop"); if (st.last_saved) showToast("success", $t("tools.common.done") as string); } catch (e) { showToast("error", errText(e)); } finally { busy = false; } }
  async function saveReplay() { busy = true; try { const p = await invoke<string>("tool_record_save_replay"); showToast("success", p); await poll(); } catch (e) { showToast("error", errText(e)); } finally { busy = false; } }
  async function saveHotkey() {
    try { const map = await invoke<Record<string, string>>("tool_hotkey_set", { action: "record", binding: hotkeyDraft }); hotkey = map.record ?? ""; showToast("success", $t("tools.common.done") as string); }
    catch (e) { showToast("error", errText(e)); }
  }
  let screens = $derived(sources.filter((s) => s.kind === "screen"));
  let audios = $derived(sources.filter((s) => s.kind === "audio"));
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{#if st.running}<span class="rec"></span> {st.replay ? $t("tools.record.buffering") : $t("tools.record.recording")} · {fmtSecs(st.seconds)}{:else}{$t("tools.record.idle")}{/if}</div>
          <div class="group-row-sub mono">{st.output ?? st.last_saved ?? ""}{#if st.error} <span class="err">{st.error}</span>{/if}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if st.running}
            {#if st.replay}<button class="btn btn-primary" type="button" disabled={busy} onclick={saveReplay}>{$t("tools.record.save_replay")} ({replaySeconds}s)</button>{/if}
            <button class="btn btn-danger" type="button" disabled={busy} onclick={stop}>{$t("tools.record.stop")}</button>
          {:else}
            <button class="btn btn-primary" type="button" disabled={busy || !screen} onclick={start}>{replay ? $t("tools.record.start_replay") : $t("tools.record.start")}</button>
          {/if}
          {#if st.last_saved}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(st.last_saved!)}>{$t("tools.common.reveal")}</button>{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.hotkey")}</div><div class="group-row-sub">{hotkey ? `${hotkey} · ${$t("tools.record.hotkey_hint")}` : $t("tools.autoclick.hotkey_none")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input mono" type="text" placeholder="CmdOrCtrl+Shift+R" bind:value={hotkeyDraft} style:width="12em" /><button class="btn btn-secondary btn-sm" type="button" onclick={saveHotkey}>{$t("tools.common.save")}</button></div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.record.screen")}</div></div>
        <div class="group-row-trailing btn-row"><select class="input" bind:value={screen} disabled={st.running}>{#each screens as s (s.id)}<option value={s.id}>{s.name}</option>{/each}</select><input class="input" type="number" min="5" max="120" bind:value={fps} style:width="5em" disabled={st.running} /><span class="dim">fps</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.record.audio")}</div><div class="group-row-sub">{$t("tools.record.audio_hint")}</div></div>
        <div class="group-row-trailing"><select class="input" bind:value={audio} disabled={st.running}><option value="">{$t("tools.record.no_audio")}</option>{#each audios as a (a.id)}<option value={a.id}>{a.name}</option>{/each}</select></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.record.replay")}</div><div class="group-row-sub">{$t("tools.record.replay_hint")}</div></div>
        <div class="group-row-trailing btn-row"><button class="toggle" class:on={replay} type="button" role="switch" aria-checked={replay} aria-label={$t("tools.record.replay")} disabled={st.running} onclick={() => (replay = !replay)}><span class="toggle-knob"></span></button>{#if replay}<input class="input" type="number" min="10" max="600" step="5" bind:value={replaySeconds} style:width="5em" disabled={st.running} /><span class="dim">s</span>{/if}</div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.record.area")}</div><div class="group-row-sub">{useArea ? `${area.x},${area.y} ${area.w}×${area.h}` : $t("tools.record.area_full")}</div></div>
        <div class="group-row-trailing btn-row">
          <button class="toggle" class:on={useArea} type="button" role="switch" aria-checked={useArea} aria-label={$t("tools.record.area")} disabled={st.running} onclick={() => (useArea = !useArea)}><span class="toggle-knob"></span></button>
          {#if useArea}<input class="input" type="number" bind:value={area.x} style:width="4.5em" /><input class="input" type="number" bind:value={area.y} style:width="4.5em" /><input class="input" type="number" bind:value={area.w} style:width="5em" /><input class="input" type="number" bind:value={area.h} style:width="5em" />{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.record.quality")} <span class="dim">· CRF {crf}</span></div><div class="group-row-sub">{$t("tools.record.quality_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input type="range" min="15" max="32" bind:value={crf} disabled={st.running} /><label class="check"><input type="checkbox" bind:checked={cursor} disabled={st.running} /> {$t("tools.record.cursor")}</label></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.record.default_dir")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" disabled={st.running} onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.record.perm")}</div></div></div>
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .err { color: var(--danger); }
  .rec { display: inline-block; width: 10px; height: 10px; border-radius: 50%; background: var(--danger); animation: blink 1s infinite; }
  @keyframes blink { 50% { opacity: 0.3; } }
  .check { display: inline-flex; align-items: center; gap: 6px; font-size: var(--text-sm); }
  .check input { accent-color: var(--accent); }
</style>
