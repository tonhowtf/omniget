<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Strip = { duration_secs: number; frames: string[] };

  let path = $state<string | null>(null);
  let strip = $state<Strip | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let start = $state(0);
  let end = $state(0);
  let exact = $state(true);
  let preview = $state<string | null>(null);
  let previewAt = $state(0);
  let result = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let asked = 0;

  const duration = $derived(strip?.duration_secs ?? 0);
  const length = $derived(Math.max(0, end - start));
  const name = $derived(path ? path.split(/[\\/]/).pop() : "");

  const errText = (e: any) => {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg ? translateBackendError(msg, $t) : $t("common.error");
  };

  function clock(s: number): string {
    const ms = Math.round(Math.max(0, s) * 1000);
    const h = Math.floor(ms / 3600000);
    const m = Math.floor((ms / 60000) % 60);
    const sec = ((ms / 1000) % 60).toFixed(2).padStart(5, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
  }

  async function pick() {
    const selected = await open({
      title: $t("downloads.tools.trim_preview.pick"),
      filters: [{ name: "Video", extensions: ["mp4", "mkv", "webm", "mov", "m4v", "avi", "ts"] }],
      multiple: false,
    });
    if (!selected || typeof selected !== "string") return;
    path = selected;
    strip = null;
    result = null;
    preview = null;
    error = null;
    loading = true;
    try {
      strip = await invoke<Strip>("trim_strip", { input: selected });
      start = 0;
      end = strip.duration_secs;
      showFrame(0);
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  // The big frame follows the handle that moved last, once it stops moving.
  function showFrame(at: number) {
    if (!path) return;
    previewAt = at;
    if (timer) clearTimeout(timer);
    timer = setTimeout(async () => {
      const ticket = ++asked;
      try {
        const img = await invoke<string>("trim_frame", { input: path, at: Math.min(at, Math.max(0, duration - 0.05)) });
        if (ticket === asked) preview = img;
      } catch {
        // The strip is still there; a missing preview frame is not worth an error.
      }
    }, 150);
  }

  function moveStart(v: number) {
    start = Math.min(v, end - 0.25);
    result = null;
    showFrame(start);
  }

  function moveEnd(v: number) {
    end = Math.max(v, start + 0.25);
    result = null;
    showFrame(end);
  }

  async function save() {
    if (!path || saving) return;
    saving = true;
    result = null;
    try {
      const r = await invoke<{ output_path: string }>("trim_export", { input: path, start, end, exact });
      result = r.output_path;
      showToast("success", $t("downloads.tools.trim_preview.saved"));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="tp" data-agent-readiness={loading || saving ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.trim_preview.lead")}</p>
  <div class="row">
    <button class="button" onclick={pick} disabled={loading || saving}>
      {loading ? $t("downloads.tools.trim_preview.loading") : $t("downloads.tools.trim_preview.pick")}
    </button>
    {#if name}<span class="dim name" title={path}>{name}</span>{/if}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if strip}
    <div class="stage">
      {#if preview}
        <img class="preview" src={preview} alt={$t("downloads.tools.trim_preview.frame_at", { time: clock(previewAt) })} />
      {:else}
        <div class="preview empty"></div>
      {/if}
      <span class="dim">{$t("downloads.tools.trim_preview.frame_at", { time: clock(previewAt) })}</span>
    </div>

    <div class="strip-wrap">
      <div class="strip">
        {#each strip.frames as f, i (i)}
          <img src={f} alt="" />
        {/each}
      </div>
      <div class="shade" style="left:0;width:{(start / duration) * 100}%"></div>
      <div class="shade" style="left:{(end / duration) * 100}%;right:0"></div>
      <input
        class="handle"
        type="range"
        min="0"
        max={duration}
        step="0.01"
        value={start}
        oninput={(e) => moveStart(Number(e.currentTarget.value))}
        aria-label={$t("downloads.tools.trim_preview.start")}
      />
      <input
        class="handle"
        type="range"
        min="0"
        max={duration}
        step="0.01"
        value={end}
        oninput={(e) => moveEnd(Number(e.currentTarget.value))}
        aria-label={$t("downloads.tools.trim_preview.end")}
      />
    </div>

    <p class="times">
      <span>{$t("downloads.tools.trim_preview.start")}: <strong>{clock(start)}</strong></span>
      <span>{$t("downloads.tools.trim_preview.end")}: <strong>{clock(end)}</strong></span>
      <span class="dim">{$t("downloads.tools.trim_preview.length", { time: clock(length) })}</span>
    </p>

    <fieldset class="modes">
      <label>
        <input type="radio" name="trim-mode" checked={exact} onchange={() => (exact = true)} />
        <span><strong>{$t("downloads.tools.trim_preview.exact")}</strong> <span class="dim">{$t("downloads.tools.trim_preview.exact_hint")}</span></span>
      </label>
      <label>
        <input type="radio" name="trim-mode" checked={!exact} onchange={() => (exact = false)} />
        <span><strong>{$t("downloads.tools.trim_preview.fast")}</strong> <span class="dim">{$t("downloads.tools.trim_preview.fast_hint")}</span></span>
      </label>
    </fieldset>

    <div class="row">
      <button class="button" onclick={save} disabled={saving}>
        {saving ? $t("downloads.tools.trim_preview.saving") : $t("downloads.tools.trim_preview.save")}
      </button>
      {#if result}
        <button class="link" onclick={() => void invoke("reveal_file", { path: result })}>{$t("downloads.tools.trim_preview.reveal")}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .tp {
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
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
  }
  .preview {
    max-width: 100%;
    height: 240px;
    object-fit: contain;
    border-radius: var(--radius-lg);
    background: #000;
  }
  .preview.empty {
    width: 100%;
  }
  .strip-wrap {
    position: relative;
    height: 56px;
  }
  .strip {
    display: flex;
    height: 100%;
    overflow: hidden;
    border-radius: var(--radius-md, 8px);
    background: #000;
  }
  .strip img {
    flex: 1;
    min-width: 0;
    height: 100%;
    object-fit: cover;
  }
  .shade {
    position: absolute;
    top: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  .handle {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    background: none;
    pointer-events: none;
    -webkit-appearance: none;
    appearance: none;
  }
  .handle::-webkit-slider-thumb {
    -webkit-appearance: none;
    pointer-events: auto;
    width: 10px;
    height: 56px;
    border-radius: 3px;
    background: var(--accent);
    cursor: ew-resize;
  }
  .handle:focus-visible::-webkit-slider-thumb {
    outline: 2px solid var(--text);
  }
  .times {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
  .modes {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: none;
    font-size: var(--text-sm);
  }
  .modes label {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    cursor: pointer;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>
