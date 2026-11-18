<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import { isImagePath } from "$lib/palette/api";
  import { importFile } from "$lib/palette/importer";
  import { imageSource } from "$lib/palette/image.svelte";
  import { TABS, workspace, type Tab } from "$lib/palette/workspace.svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import ProjectBar from "$components/palette/ProjectBar.svelte";
  import PickerPanel from "$components/palette/PickerPanel.svelte";
  import ScalesPanel from "$components/palette/ScalesPanel.svelte";
  import ContrastPanel from "$components/palette/ContrastPanel.svelte";
  import GradientPanel from "$components/palette/GradientPanel.svelte";
  import ImagePanel from "$components/palette/ImagePanel.svelte";
  import ExportPanel from "$components/palette/ExportPanel.svelte";
  import StatusIcon from "$components/converter/StatusIcon.svelte";

  let dragging = $state(false);
  let dropNotice = $state<string | null>(null);
  let tabEls: HTMLButtonElement[] = $state([]);
  let firstInput = $state<HTMLInputElement | null>(null);
  let firstError = $state<string | null>(null);

  function untitledPalette(n: number): string {
    return $t("palette.scales.untitled", { num: n }) as string;
  }

  async function firstImport(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    const res = await importFile(file, $t("palette.untitled_project") as string, untitledPalette);
    if (!res.ok) {
      firstError = $t(`palette.export.import_${res.reason}`) as string;
      return;
    }
    firstError = null;
    workspace.addProject(res.project);
    workspace.tab = "scales";
  }

  let ready = $derived(workspace.phase === "ready" && workspace.project !== null);

  function seedName(key: string): string {
    return $t(`palette.seed.${key}`) as string;
  }

  function newProject(): string {
    const n = workspace.file.projects.length + 1;
    const name = n === 1 ? ($t("palette.project.first_name") as string) : ($t("palette.project.nth_name", { num: n }) as string);
    workspace.createProject(name, seedName);
    return name;
  }

  function selectTab(tab: Tab) {
    workspace.tab = tab;
  }

  function tabKey(e: KeyboardEvent, index: number) {
    const moves: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1 };
    let next = -1;
    if (e.key in moves) next = (index + moves[e.key] + TABS.length) % TABS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    workspace.tab = TABS[next];
    tabEls[next]?.focus();
  }

  function onDrop(paths: string[]) {
    const image = paths.find(isImagePath);
    if (!image) {
      dropNotice = $t("palette.drop_not_image") as string;
      return;
    }
    dropNotice = null;
    if (workspace.phase !== "ready") return;
    if (!workspace.project) {
      const name = newProject();
      workspace.showToast($t("palette.drop_created_project", { name }) as string, "success");
    }
    workspace.tab = "image";
    void imageSource.read(image);
  }

  function keydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !e.shiftKey && workspace.removed && workspace.toast?.undo) {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
      e.preventDefault();
      workspace.undo();
    }
  }

  onMount(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];
    const keep = (fn: () => void) => {
      if (disposed) fn();
      else cleanups.push(fn);
    };
    if (workspace.phase !== "ready") void workspace.load($t("palette.untitled_project") as string, untitledPalette);
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dragging = workspace.phase === "ready";
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (p.paths?.length) onDrop(p.paths);
          }
        })
        .then(keep)
        .catch(() => {});
    } catch {
      dragging = false;
    }
    const flushNow = () => void workspace.flush();
    const flushOnHide = () => {
      if (document.visibilityState === "hidden") flushNow();
    };
    document.addEventListener("visibilitychange", flushOnHide);
    window.addEventListener("pagehide", flushNow);
    window.addEventListener("beforeunload", flushNow);
    cleanups.push(() => {
      document.removeEventListener("visibilitychange", flushOnHide);
      window.removeEventListener("pagehide", flushNow);
      window.removeEventListener("beforeunload", flushNow);
    });
    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
      void workspace.flush();
    };
  });
</script>

<svelte:window onkeydown={keydown} />

<section class="pl" aria-busy={workspace.phase === "loading"}>
  <header class="pl-head">
    <NavIcon icon="palette" size={56} />
    <div class="pl-head-copy">
      <h1>{$t("palette.title")}</h1>
      <p class="pl-subtitle">{$t("palette.subtitle")}</p>
    </div>
  </header>

  {#if workspace.movedAside}
    <div class="pl-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{$t("palette.moved_aside", { path: workspace.movedAside })}</p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (workspace.movedAside = null)}>{$t("palette.dismiss")}</button>
    </div>
  {/if}
  {#if dropNotice}
    <div class="pl-notice" role="alert">
      <StatusIcon name="warning" />
      <p>{dropNotice}</p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (dropNotice = null)}>{$t("palette.dismiss")}</button>
    </div>
  {/if}

  {#if workspace.phase === "loading"}
    <div class="pl-state">
      <span class="spinner" aria-hidden="true"></span>
      <p role="status">{$t("palette.loading")}</p>
    </div>
  {:else if workspace.phase === "error"}
    <div class="pl-state" role="alert">
      <StatusIcon name="failed" size={22} />
      <h2>{$t("palette.load_error_title")}</h2>
      <p>{workspace.loadError}</p>
      <p class="pl-dim">{$t("palette.load_error_fix")}</p>
      <button type="button" class="btn btn-primary" onclick={() => workspace.load($t("palette.untitled_project") as string, untitledPalette)}>{$t("palette.retry_load")}</button>
    </div>
  {:else if !workspace.project}
    <div class="pl-state pl-first">
      <div class="pl-first-art" aria-hidden="true">
        {#each ["var(--blue)", "var(--teal)", "var(--success)", "var(--warning)", "var(--orange)", "var(--red)", "var(--purple)"] as c, i (i)}<span style:background={c}></span>{/each}
      </div>
      <h2>{$t("palette.first_title")}</h2>
      <p>{$t("palette.first_body")}</p>
      <button type="button" class="btn btn-primary" onclick={newProject}>{$t("palette.first_create")}</button>
      <input bind:this={firstInput} class="pl-hidden" type="file" accept=".json,application/json" tabindex="-1" aria-hidden="true" onchange={firstImport} />
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => firstInput?.click()}>{$t("palette.first_import")}</button>
      {#if firstError}<p class="pl-first-error" role="alert">{firstError}</p>{/if}
    </div>
  {:else}
    <ProjectBar onnew={newProject} />

    <div class="segmented segmented-lg pl-tabs" role="tablist" aria-label={$t("palette.tabs_label")}>
      {#each TABS as tab, i (tab)}
        <button
          bind:this={tabEls[i]}
          type="button"
          class="segmented-btn"
          role="tab"
          id="pl-tab-{tab}"
          aria-selected={workspace.tab === tab}
          aria-controls="pl-panel"
          tabindex={workspace.tab === tab ? 0 : -1}
          onclick={() => selectTab(tab)}
          onkeydown={(e) => tabKey(e, i)}
        >
          {$t(`palette.tab.${tab}`)}
        </button>
      {/each}
    </div>

    <div class="pl-panel" id="pl-panel" role="tabpanel" aria-labelledby="pl-tab-{workspace.tab}">
      {#if workspace.tab === "color"}
        <PickerPanel />
      {:else if workspace.tab === "scales"}
        <ScalesPanel />
      {:else if workspace.tab === "contrast"}
        <ContrastPanel />
      {:else if workspace.tab === "gradients"}
        <GradientPanel />
      {:else if workspace.tab === "image"}
        <ImagePanel {dragging} />
      {:else}
        <ExportPanel />
      {/if}
    </div>
  {/if}

  {#if workspace.toast}
    <div class="pl-toast pl-toast-{workspace.toast.tone}" role="status" aria-live="polite">
      <p>{workspace.toast.text}</p>
      {#if workspace.toast.undo}
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => workspace.undo()}>{$t("palette.undo")}</button>
      {/if}
      <button type="button" class="btn btn-ghost btn-sm pl-toast-x" aria-label={$t("palette.dismiss")} onclick={() => workspace.dismissToast()}>
        <svg viewBox="0 0 12 12" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
      </button>
    </div>
  {/if}

  {#if dragging && (!ready || workspace.tab !== "image" || imageSource.phase !== "empty")}
    <div class="pl-dropping" aria-hidden="true">
      <p>{$t("palette.drop_here")}</p>
    </div>
  {/if}
</section>

<style>
  .pl {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .pl-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .pl-head-copy {
    min-width: 0;
  }

  .pl-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .pl-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .pl-notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-notice p {
    flex: 1;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .pl-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 320px;
    padding: var(--space-7) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    text-align: center;
  }

  .pl-state h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .pl-state p {
    margin: 0;
    max-width: 460px;
    font-size: var(--text-base);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }

  .pl-state .pl-dim {
    font-size: var(--text-sm);
  }

  .pl-first-art {
    display: flex;
    gap: 4px;
  }

  .pl-first-art span {
    width: 18px;
    height: 44px;
    border-radius: var(--radius-sm);
  }

  .pl-hidden {
    display: none;
  }

  .pl-state .pl-first-error {
    color: var(--error);
    font-size: var(--text-sm);
  }

  .pl-tabs {
    align-self: flex-start;
    flex-wrap: wrap;
  }

  .pl-panel {
    min-width: 0;
  }

  .pl-toast {
    position: sticky;
    bottom: var(--space-4);
    z-index: 3;
    align-self: center;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 560px;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--popup-bg);
    box-shadow: 0 8px 24px rgba(var(--shadow-ink), var(--elev-alpha-3)), inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-toast p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .pl-toast-error p {
    color: var(--error);
  }

  .pl-toast-x {
    width: var(--control-h);
    padding: 0;
    justify-content: center;
  }

  .pl-dropping {
    position: absolute;
    inset: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    pointer-events: none;
  }

  .pl-dropping p {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }
</style>
