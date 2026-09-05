<script lang="ts">
  /** Autoclicker (estudo 14, Blur AutoClicker): CPS exato, faixa aleatória, limites, posição fixa e atalho global. */
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/tools/rt";

  type State = { running: boolean; clicks: number; elapsed_ms: number; error: string | null };
  let cps = $state(10);
  let jitter = $state(0);
  let button = $state<"left" | "right" | "middle">("left");
  let double = $state(false);
  let maxClicks = $state(0);
  let maxSeconds = $state(0);
  let fixed = $state(false);
  let pos = $state<[number, number]>([0, 0]);
  let holdMs = $state(0);
  let delay = $state(3);
  let st = $state<State>({ running: false, clicks: 0, elapsed_ms: 0, error: null });
  let hotkey = $state("");
  let hotkeyDraft = $state("");
  let timer: ReturnType<typeof setInterval> | null = null;
  let unlisten: (() => void) | null = null;

  async function poll() { try { st = await invoke<State>("tool_autoclick_state"); } catch { /* ignore */ } }
  onMount(async () => {
    const map = await invoke<Record<string, string>>("tool_hotkeys_get");
    hotkey = map.autoclick ?? ""; hotkeyDraft = hotkey;
    await poll();
    timer = setInterval(poll, 500);
    unlisten = await listen<{ running: boolean; error?: string }>("tool-autoclick", (e) => { if (e.payload.error) showToast("error", e.payload.error); poll(); });
  });
  onDestroy(() => { if (timer) clearInterval(timer); unlisten?.(); });

  async function start() {
    try {
      st = await invoke<State>("tool_autoclick_start", { opts: { cps, jitter_pct: jitter, button, double, max_clicks: maxClicks, max_seconds: maxSeconds, position: fixed ? pos : null, hold_ms: holdMs, start_delay: delay } });
    } catch (e) { showToast("error", errText(e)); }
  }
  async function stop() { st = await invoke<State>("tool_autoclick_stop"); }
  async function grab() { const p = await invoke<[number, number] | null>("tool_autoclick_mouse"); if (p) { pos = p; fixed = true; } }
  async function saveHotkey() {
    try { const map = await invoke<Record<string, string>>("tool_hotkey_set", { action: "autoclick", binding: hotkeyDraft }); hotkey = map.autoclick ?? ""; showToast("success", $t("tools.common.done") as string); }
    catch (e) { showToast("error", errText(e)); }
  }
  const fmt = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}`;
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{st.running ? $t("tools.autoclick.running") : $t("tools.autoclick.idle")}</div>
          <div class="group-row-sub">{st.clicks} {$t("tools.autoclick.clicks")} · {fmt(st.elapsed_ms)}{#if st.error} · <span class="err">{st.error}</span>{/if}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if st.running}<button class="btn btn-danger" type="button" onclick={stop}>{$t("tools.autoclick.stop")}</button>{:else}<button class="btn btn-primary" type="button" onclick={start}>{$t("tools.autoclick.start")}</button>{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.hotkey")}</div><div class="group-row-sub">{hotkey ? `${hotkey} · ${$t("tools.autoclick.hotkey_hint")}` : $t("tools.autoclick.hotkey_none")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input mono" type="text" placeholder="F6 · CmdOrCtrl+Shift+C" bind:value={hotkeyDraft} style:width="12em" /><button class="btn btn-secondary btn-sm" type="button" onclick={saveHotkey}>{$t("tools.common.save")}</button></div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.cps")} <span class="dim">· {cps}/s{#if jitter} ± {jitter}%{/if}</span></div><div class="group-row-sub">{$t("tools.autoclick.cps_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0.1" max="1000" step="0.1" bind:value={cps} style:width="6em" /><input class="input" type="number" min="0" max="90" bind:value={jitter} style:width="5em" title="jitter %" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.button")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={button}><option value="left">{$t("tools.autoclick.left")}</option><option value="right">{$t("tools.autoclick.right")}</option><option value="middle">{$t("tools.autoclick.middle")}</option></select>
          <label class="check"><input type="checkbox" bind:checked={double} /> {$t("tools.autoclick.double")}</label>
          <input class="input" type="number" min="0" max="2000" bind:value={holdMs} style:width="5em" title={$t("tools.autoclick.hold")} /><span class="dim">ms</span>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.limits")}</div><div class="group-row-sub">{$t("tools.autoclick.limits_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" bind:value={maxClicks} style:width="7em" title={$t("tools.autoclick.clicks")} /><input class="input" type="number" min="0" bind:value={maxSeconds} style:width="6em" title="s" /><span class="dim">s</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.position")}</div><div class="group-row-sub">{fixed ? `${pos[0]}, ${pos[1]}` : $t("tools.autoclick.position_cursor")}</div></div>
        <div class="group-row-trailing btn-row"><label class="check"><input type="checkbox" bind:checked={fixed} /> {$t("tools.autoclick.fixed")}</label><button class="btn btn-secondary btn-sm" type="button" onclick={grab}>{$t("tools.autoclick.grab")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.autoclick.delay")}</div><div class="group-row-sub">{$t("tools.autoclick.delay_hint")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" max="60" bind:value={delay} style:width="5em" /><span class="dim">s</span></div>
      </div>
      <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{$t("tools.autoclick.perm")}</div></div></div>
    </div>
  </section>
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .err { color: var(--danger); }
  .check { display: inline-flex; align-items: center; gap: 6px; font-size: var(--text-sm); }
  .check input { accent-color: var(--accent); }
</style>
