<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { pluralKey } from "$lib/fonts/text";
  import { t } from "$lib/i18n";
  import * as api from "$lib/fonts/api";
  import { hasFullFiles, pickSubset, plannedCount, subsetKey, subsetOptions } from "$lib/fonts/install";
  import { licenseName, licenseUrl } from "$lib/fonts/license";
  import type { CatalogFont, FontDetails, InstallOutcome, InstallProgress } from "$lib/fonts/types";
  import StatusIcon, { type StatusIconName } from "#components/converter/StatusIcon.svelte";

  let {
    font,
    alreadyInstalled,
    userDir,
    oninstalled,
  }: {
    font: CatalogFont;
    alreadyInstalled: boolean;
    userDir: string | null;
    oninstalled: () => void;
  } = $props();

  type Phase = "idle" | "running" | "done" | "error";

  let details = $state<FontDetails | null>(null);
  let detailsError = $state<string | null>(null);
  let loadingDetails = $state(false);
  let subset = $state("latin");
  let phase = $state<Phase>("idle");
  let mode = $state<"install" | "download">("install");
  let progress = $state<{ index: number; total: number; file: string; done: number } | null>(null);
  let outcome = $state<InstallOutcome | null>(null);
  let outcomeFamily = $state("");
  let outcomeSubset = $state("");
  let runError = $state<string | null>(null);
  let stopping = $state(false);
  let confirming = $state(false);
  let jobId: string | null = null;
  let loadedFor = "";

  let open = $derived(font.license_class === "open");
  let full = $derived(hasFullFiles(details));
  let options = $derived(subsetOptions(details));
  let fileCount = $derived(plannedCount(details, subset));
  let busy = $derived(phase === "running");
  let installReason = $derived.by(() => {
    if (!open) return $t("fonts.install_blocked_license") as string;
    if (detailsError) return $t("fonts.install_blocked_details") as string;
    if (!details) return $t("fonts.loading_files") as string;
    if (fileCount === 0) return $t("fonts.install_blocked_subset") as string;
    return null;
  });
  let finished = $derived(phase === "done" && !!outcome && outcome.failed.length === 0 && !outcome.cancelled && (outcome.saved.length > 0 || outcome.skipped.length > 0));
  let firstSaved = $derived(outcome?.saved[0] ?? null);
  let progressLabel = $derived(progress ? ($t("fonts.install_progress", { file: progress.file, index: progress.index + 1, total: progress.total }) as string) : "");
  let installPrimary = $derived(!alreadyInstalled && !finished);

  function scriptName(s: string): string {
    const key = subsetKey(s);
    return key ? ($t(key) as string) : s;
  }

  async function loadDetails(id: string) {
    loadingDetails = true;
    detailsError = null;
    try {
      const d = await api.details(id);
      if (font.id !== id) return;
      details = d;
      subset = pickSubset(d);
    } catch (e) {
      if (font.id === id) detailsError = api.errorText(e);
    } finally {
      if (font.id === id) loadingDetails = false;
    }
  }

  $effect(() => {
    const id = font.id;
    if (id === loadedFor) return;
    loadedFor = id;
    if (jobId) void api.cancel(jobId).catch(() => {});
    jobId = null;
    details = null;
    detailsError = null;
    loadingDetails = false;
    outcome = null;
    runError = null;
    confirming = false;
    stopping = false;
    phase = "idle";
    progress = null;
    if (font.license_class === "open") void loadDetails(id);
  });

  function newJobId(): string {
    return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `ty-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  function askInstall() {
    if (alreadyInstalled && !confirming) {
      confirming = true;
      return;
    }
    void run("install");
  }

  async function run(target: "install" | "download") {
    if (busy || installReason) return;
    confirming = false;
    let outDir: string | null = null;
    if (target === "download") {
      const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
      if (typeof picked !== "string") return;
      outDir = picked;
    }
    const family = font.family;
    const chosen = subset;
    mode = target;
    phase = "running";
    stopping = false;
    outcome = null;
    runError = null;
    progress = { index: 0, total: fileCount, file: "", done: 0 };
    const id = newJobId();
    jobId = id;
    try {
      const result = await api.install(id, font.id, chosen, outDir);
      if (result.installed && result.saved.length > 0) oninstalled();
      if (jobId !== id) return;
      outcome = result;
      outcomeFamily = family;
      outcomeSubset = chosen;
      phase = "done";
    } catch (e) {
      if (jobId !== id) return;
      runError = api.errorText(e);
      phase = "error";
    } finally {
      if (jobId === id) {
        jobId = null;
        progress = null;
        stopping = false;
      }
    }
  }

  async function stop() {
    if (!jobId || stopping) return;
    stopping = true;
    await api.cancel(jobId).catch(() => {});
  }

  function onProgress(event: InstallProgress) {
    if (!jobId || event.job_id !== jobId || !progress) return;
    progress.total = event.total;
    progress.index = event.index;
    progress.file = event.file_name;
    if (event.state !== "downloading") progress.done = event.index + 1;
  }

  function showInFolder() {
    const target = firstSaved ?? outcome?.dir ?? null;
    if (target) void api.reveal(target).catch(() => {});
  }

  let resultTone = $derived.by((): StatusIconName => {
    if (!outcome) return "done";
    if (outcome.failed.length > 0) return outcome.saved.length > 0 ? "warning" : "failed";
    if (outcome.cancelled) return "stopped";
    if (outcome.saved.length === 0) return "skipped";
    return "done";
  });

  let resultTitle = $derived.by(() => {
    if (!outcome) return "";
    const n = outcome.saved.length;
    const name = outcomeFamily;
    if (outcome.cancelled) return $t(pluralKey("fonts.result_cancelled", n), { count: n }) as string;
    if (outcome.failed.length > 0 && n === 0) return $t("fonts.result_failed", { name }) as string;
    if (outcome.failed.length > 0) return $t("fonts.result_partial", { count: n, total: n + outcome.failed.length }) as string;
    if (n === 0) return $t("fonts.result_nothing", { name }) as string;
    return (outcome.installed ? $t(pluralKey("fonts.result_installed", n), { name, count: n }) : $t(pluralKey("fonts.result_downloaded", n), { name, count: n })) as string;
  });

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void api
      .onInstallProgress(onProgress)
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      if (jobId) void api.cancel(jobId).catch(() => {});
      jobId = null;
    };
  });
</script>

<section class="ty-inst" aria-labelledby="ty-inst-title">
  <div class="ty-inst-head">
    <h3 id="ty-inst-title">{$t("fonts.get_title")}</h3>
    <span class="ty-inst-license">
      <StatusIcon name={open ? "done" : "warning"} size={14} />
      {#if licenseUrl(font.license)}
        <a href={licenseUrl(font.license)} target="_blank" rel="noreferrer">{licenseName(font.license)}</a>
      {:else}
        <span>{font.license ? licenseName(font.license) : $t("fonts.license_unknown")}</span>
      {/if}
    </span>
  </div>

  {#if !open}
    <p class="ty-inst-copy">{$t("fonts.license_other_body", { license: font.license || "?" })}</p>
  {:else}
    <p class="ty-inst-copy">
      {$t(pluralKey("fonts.install_body", fileCount), { count: fileCount, dir: userDir ?? $t("fonts.user_folder") })}
      {#if details}{" "}{full ? $t("fonts.full_note") : $t("fonts.subset_only_note", { script: scriptName(subset) })}{/if}
    </p>

    {#if alreadyInstalled}
      <div class="ty-inst-note">
        <StatusIcon name="warning" size={14} />
        <span>{$t("fonts.duplicate_body", { name: font.family })}</span>
      </div>
    {/if}

    {#if detailsError}
      <div class="ty-inst-note" role="alert">
        <StatusIcon name="warning" size={14} />
        <span>{$t("fonts.details_error", { error: detailsError })}</span>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => loadDetails(font.id)}>{$t("fonts.retry")}</button>
      </div>
    {/if}

    {#if details && !full && options.length > 1}
      <label class="ty-inst-field">
        <span class="field-label">{$t("fonts.subset")}</span>
        <select class="ty-select" bind:value={subset} disabled={busy}>
          {#each options as s (s)}
            <option value={s}>{scriptName(s)}</option>
          {/each}
        </select>
        <span class="field-hint">{$t("fonts.subset_hint")}</span>
      </label>
    {/if}
  {/if}

  <p class="ty-inst-live" role="status" aria-live="polite">{busy ? progressLabel : resultTitle}</p>

  {#if phase === "running" && progress}
    <div class="ty-inst-progress">
      <div class="progress" aria-hidden="true"><div class="progress-fill" style:width="{progress.total ? (progress.done / progress.total) * 100 : 0}%"></div></div>
      <span class="ty-inst-progress-label">{progressLabel || $t("fonts.install_starting")}</span>
    </div>
  {/if}

  {#if phase === "done" && outcome}
    <div class="ty-inst-result">
      <StatusIcon name={resultTone} size={18} />
      <div class="ty-inst-result-copy">
        <p class="ty-inst-result-title">{resultTitle}</p>
        {#if outcome.skipped.length > 0}
          <p>{$t(pluralKey("fonts.result_skipped", outcome.skipped.length), { count: outcome.skipped.length })}</p>
        {/if}
        {#each outcome.failed as f (f.file_name)}
          <p class="ty-inst-fail"><strong>{f.file_name}</strong> {f.error}</p>
        {/each}
        {#if outcome.failed.length > 0}
          <p>{$t("fonts.result_failed_fix")}</p>
        {/if}
        {#if outcome.coverage === "subset" && full && outcome.saved.length > 0}
          <p>{$t("fonts.result_subset_fallback", { script: scriptName(outcomeSubset) })}</p>
        {/if}
        {#if outcome.installed && outcome.saved.length > 0}
          <p>{outcome.needs_restart ? $t("fonts.result_restart") : $t("fonts.result_ready")}</p>
        {/if}
      </div>
    </div>
  {/if}

  {#if phase === "error" && runError}
    <div class="ty-inst-note" role="alert">
      <StatusIcon name="failed" size={14} />
      <span>{$t("fonts.install_error", { error: runError })}</span>
    </div>
  {/if}

  {#if confirming && !busy}
    <div class="ty-inst-confirm" role="group" aria-labelledby="ty-inst-confirm-text">
      <p id="ty-inst-confirm-text">{$t("fonts.duplicate_confirm", { name: font.family })}</p>
      <div class="ty-inst-actions">
        <button type="button" class="btn btn-ghost" onclick={() => (confirming = false)}>{$t("fonts.duplicate_keep")}</button>
        <button type="button" class="btn btn-secondary" disabled={!!installReason} onclick={askInstall}>{$t("fonts.duplicate_install")}</button>
      </div>
    </div>
  {/if}

  <div class="ty-inst-actions">
    {#if busy}
      <button type="button" class="btn btn-secondary" disabled={stopping} onclick={stop}>{stopping ? $t("fonts.stopping") : $t("fonts.stop")}</button>
    {:else}
      <button type="button" class="btn {alreadyInstalled && !finished ? 'btn-primary' : 'btn-secondary'}" disabled={!!installReason} title={installReason ?? undefined} onclick={() => run("download")}>{$t("fonts.download_files")}</button>
      {#if !confirming}
        <button type="button" class="btn {installPrimary ? 'btn-primary' : 'btn-secondary'}" disabled={!!installReason} title={installReason ?? undefined} onclick={askInstall}>
          {#if loadingDetails}<span class="spinner ty-inst-spin" aria-hidden="true"></span>{/if}
          {#if phase === "done" && mode === "install" && outcome?.failed.length}{$t("fonts.install_retry")}{:else if alreadyInstalled}{$t("fonts.install_again", { name: font.family })}{:else}{$t("fonts.install", { name: font.family })}{/if}
        </button>
      {/if}
      {#if phase === "done" && outcome && (outcome.saved.length > 0 || outcome.skipped.length > 0)}
        <button type="button" class="btn {finished ? 'btn-primary' : 'btn-secondary'}" onclick={showInFolder}>{$t("fonts.show_in_folder")}</button>
      {/if}
    {/if}
  </div>
  {#if installReason && !busy && !loadingDetails && open}
    <p class="field-hint ty-inst-reason">{installReason}</p>
  {/if}
</section>

<style>
  .ty-inst {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-inst-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ty-inst-head h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .ty-inst-license {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .ty-inst-license a {
    color: var(--accent-text);
  }

  .ty-inst-copy {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .ty-inst-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-width: 320px;
  }

  .ty-inst-note {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ty-inst-live {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .ty-inst-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ty-inst-progress-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  .ty-inst-result {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .ty-inst-result-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .ty-inst-result-copy p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .ty-inst-result-copy .ty-inst-result-title {
    font-weight: 600;
    color: var(--text);
  }

  .ty-inst-fail strong {
    font-weight: 600;
    color: var(--text);
  }

  .ty-inst-confirm {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-hi);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-inst-confirm p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ty-inst-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .ty-inst-reason {
    margin: 0;
    text-align: right;
  }

  .ty-inst-spin {
    width: 12px;
    height: 12px;
  }

  :global(.ty-select) {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: var(--hairline) solid var(--input-border);
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
  }

  :global(.ty-select:focus-visible) {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
