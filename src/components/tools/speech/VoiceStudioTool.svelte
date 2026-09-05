<script lang="ts">
  /** Ponte com o VoiceStudio (lote 2): clonar voz, criar voz por descrição e isolar a voz (Demucs). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openPath, openUrl, pickDir, pickFile, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  let { mode = "clone" }: { mode?: "clone" | "design" | "isolate" } = $props();

  type Profile = { id: string; name: string; kind: string; language: string; instruct: string };
  type Status = { base_url: string; running: boolean; installed: boolean; app_path: string | null; version: string | null; engine: string | null; profiles: Profile[] };

  const RELEASES = "https://github.com/debpalash/VoiceStudio/releases/latest";
  let status = $state<Status | null>(null);
  let baseUrl = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let outputs = $state<string[]>([]);
  let outDir = $state("");
  // clone
  let sample = $state("");
  let sampleText = $state("");
  let profileId = $state("");
  let saveAs = $state("");
  let text = $state("");
  let language = $state("");
  let speed = $state(1);
  // design
  let description = $state("");
  let designInfo = $state<{ instruct: string; matched: string[]; unmatched: string[] } | null>(null);
  // isolate
  let input = $state("");
  let instrumental = $state(true);

  let unlisten: (() => void) | null = null;
  onMount(async () => {
    unlisten = await onToolProgress((p) => { if (p.id === "voicestudio") progress = p; });
    await refresh();
  });
  onDestroy(() => unlisten?.());
  async function refresh() { try { status = await invoke<Status>("tool_vs_status", { baseUrl }); } catch (e) { showToast("error", errText(e)); } }
  async function launch() { try { await invoke("tool_vs_launch"); showToast("info", $t("tools.vs.launching") as string); setTimeout(refresh, 6000); } catch (e) { showToast("error", errText(e)); } }

  async function run() {
    if (busy) return;
    busy = true; outputs = []; progress = null; designInfo = null;
    try {
      if (mode === "clone") {
        if (!text.trim() || (!sample && !profileId)) return;
        const r = await invoke<{ output: string; profile_id: string | null }>("tool_vs_clone", { opts: { base_url: baseUrl, sample, sample_text: sampleText, profile_id: profileId, save_as: saveAs, text, language, speed, output_dir: outDir } });
        outputs = [r.output]; if (r.profile_id && saveAs) { saveAs = ""; profileId = r.profile_id; await refresh(); }
      } else if (mode === "design") {
        if (!text.trim() || !description.trim()) return;
        const r = await invoke<{ output: string; instruct: string; matched: string[]; unmatched: string[]; profile_id: string | null }>("tool_vs_design", { opts: { base_url: baseUrl, description, text, language, save_as: saveAs, output_dir: outDir } });
        outputs = [r.output]; designInfo = r; if (r.profile_id) { saveAs = ""; await refresh(); }
      } else {
        if (!input) return;
        outputs = await invoke<string[]>("tool_vs_isolate", { baseUrl, input, outputDir: outDir, instrumental });
      }
      showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; progress = null; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">VoiceStudio {#if status?.running}<span class="tag tag-success">{$t("tools.vs.running")}{#if status.version} · {status.version}{/if}</span>{:else if status?.installed}<span class="tag tag-warning">{$t("tools.vs.not_running")}</span>{:else if status}<span class="tag">{$t("tools.common.not_installed")}</span>{/if}</div>
          <div class="group-row-sub">{status?.engine ? `${$t("tools.vs.engine")}: ${status.engine} · ` : ""}{status?.app_path ?? $t("tools.vs.hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if status && !status.installed}<button class="btn btn-primary btn-sm" type="button" onclick={() => openUrl(RELEASES)}>{$t("tools.common.download")}</button>{/if}
          {#if status?.installed && !status.running}<button class="btn btn-primary btn-sm" type="button" onclick={launch}>{$t("tools.common.open")}</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={refresh}>{$t("tools.common.refresh")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.base_url")}</div><div class="group-row-sub">{$t("tools.vs.base_url_hint")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" placeholder="http://127.0.0.1:3900" bind:value={baseUrl} onchange={refresh} style:width="14em" /></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      {#if mode === "clone"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.profile")}</div><div class="group-row-sub">{$t("tools.vs.profile_hint")}</div></div>
          <div class="group-row-trailing"><select class="input" bind:value={profileId}><option value="">{$t("tools.vs.profile_none")}</option>{#each status?.profiles ?? [] as p (p.id)}<option value={p.id}>{p.name} ({p.kind})</option>{/each}</select></div>
        </div>
        {#if !profileId}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.sample")}</div><div class="group-row-sub mono">{sample ? baseName(sample) : $t("tools.vs.sample_hint")}</div></div>
            <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.media); if (f) sample = f; }}>{$t("tools.common.choose")}</button></div>
          </div>
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.sample_text")} <span class="dim">· {$t("tools.common.optional")}</span></div></div>
            <div class="group-row-trailing"><input class="input" type="text" bind:value={sampleText} style:width="18em" /></div>
          </div>
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.save_as")} <span class="dim">· {$t("tools.common.optional")}</span></div><div class="group-row-sub">{$t("tools.vs.save_as_hint")}</div></div>
            <div class="group-row-trailing"><input class="input" type="text" bind:value={saveAs} style:width="12em" /></div>
          </div>
        {/if}
      {:else if mode === "design"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.description")}</div><div class="group-row-sub">{$t("tools.vs.description_hint")}</div></div>
        </div>
        <div class="group-row"><textarea class="input area" rows="2" bind:value={description} placeholder={$t("tools.vs.description_placeholder")}></textarea></div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.save_as")} <span class="dim">· {$t("tools.common.optional")}</span></div></div>
          <div class="group-row-trailing"><input class="input" type="text" bind:value={saveAs} style:width="12em" /></div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.file")}</div><div class="group-row-sub mono">{input ? baseName(input) : $t("tools.vs.isolate_hint")}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.media); if (f) input = f; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.vs.instrumental")}</div><div class="group-row-sub">{$t("tools.vs.instrumental_hint")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={instrumental} type="button" role="switch" aria-checked={instrumental} aria-label={$t("tools.vs.instrumental")} onclick={() => (instrumental = !instrumental)}><span class="toggle-knob"></span></button></div>
        </div>
      {/if}
      {#if mode !== "isolate"}
        <div class="group-row"><textarea class="input area" rows="4" bind:value={text} placeholder={$t("tools.vs.text_placeholder")}></textarea></div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.dictation.language")}{#if mode === "clone"} <span class="dim">· {$t("tools.vs.speed")} {speed}×</span>{/if}</div></div>
          <div class="group-row-trailing btn-row"><input class="input mono" type="text" bind:value={language} placeholder="auto" style:width="6em" />{#if mode === "clone"}<input type="range" min="0.5" max="2" step="0.05" bind:value={speed} />{/if}</div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{progress ? $t(`tools.vs.stage_${progress.stage}`) : "…"}</div><div class="progress"><div class="progress-fill indeterminate"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !status?.running} onclick={run}>{busy ? $t("tools.common.working") : $t(`tools.vs.run_${mode}`)}</button></div>
      </div>
    </div>
  </section>

  {#if designInfo}
    <section><div class="group">
      <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.vs.instruct")}</div><div class="group-row-sub mono">{designInfo.instruct || "—"}</div>{#if designInfo.unmatched.length}<div class="group-row-sub">{$t("tools.vs.unmatched")}: {designInfo.unmatched.join(", ")}</div>{/if}</div></div>
    </div></section>
  {/if}
  {#if outputs.length}
    <section><div class="group">
      {#each outputs as o (o)}
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{baseName(o)}</div><div class="group-row-sub mono">{o}</div></div><div class="group-row-trailing btn-row"><button class="btn btn-primary btn-sm" type="button" onclick={() => openPath(o)}>{$t("tools.common.play")}</button><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(o)}>{$t("tools.common.reveal")}</button></div></div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .area { width: 100%; resize: vertical; font: inherit; }
  .progress-fill.indeterminate { width: 40%; animation: slide 1.2s ease-in-out infinite alternate; }
  @keyframes slide { from { margin-left: 0; } to { margin-left: 60%; } }
</style>
