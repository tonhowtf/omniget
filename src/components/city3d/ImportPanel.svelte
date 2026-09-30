<script lang="ts">
  /**
   * Import and export of 3D assets (M6): pick a GLB/glTF/VRM/OBJ or an image,
   * choose the class, the licence and "fit to style"; the Rust normalizer
   * brings it to the grid, the budget and the palette, and the result shows
   * in a turntable preview. Below, the local assets with export (GLB, .vox)
   * and remove.
   */
  import { onDestroy, onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    ASSET_CLASSES,
    IMAGE_EXTENSIONS,
    IMAGE_MODES,
    LICENSE_KINDS,
    MODEL_EXTENSIONS,
    assetThumbUrl,
    autorig,
    autorigConfigure,
    autorigInstallCli,
    autorigInstallWeights,
    createPreview,
    generate3d,
    providersStatus,
    setTripoKey,
    deleteAsset,
    importFile,
    listAssets,
    loadAsset,
    saveAssetGlb,
    saveAssetVox,
    type AssetClass,
    type AssetMeta,
    type ImageMode,
    type LicenseKind,
    type Preview,
    type ProvidersStatus,
  } from "$lib/city3d/assets";

  let { palette, onimported }: { palette?: Record<string, string>; onimported?: (a: AssetMeta) => void } = $props();

  let path = $state<string | null>(null);
  let klass = $state<AssetClass>("prop");
  let licenseKind = $state<LicenseKind>("own");
  let author = $state("");
  let licenseUrl = $state("");
  let fitStyle = $state(false);
  let imageMode = $state<ImageMode>("papercraft");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let last = $state<AssetMeta | null>(null);
  let assets = $state<AssetMeta[]>([]);
  let thumbs = $state<Record<string, string>>({});
  let selected = $state<string | null>(null);
  let voxelMm = $state(125);

  let canvas = $state<HTMLCanvasElement | null>(null);
  let preview: Preview | null = null;
  let ro: ResizeObserver | null = null;

  const fileName = $derived(path ? path.split(/[\\/]/).pop() ?? path : "");
  const ext = $derived(fileName.includes(".") ? fileName.split(".").pop()!.toLowerCase() : "");
  const isImage = $derived(IMAGE_EXTENSIONS.includes(ext));
  const isVrm = $derived(ext === "vrm");
  const needsAuthor = $derived(licenseKind === "cc-by");
  const canImport = $derived(!!path && !busy && (!needsAuthor || author.trim().length > 0) && (licenseKind !== "vrm" || isVrm));

  function kb(n: number): string {
    return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
  }

  async function pick() {
    const p = await open({
      multiple: false,
      filters: [
        { name: $t("city3d.import.filter_models") as string, extensions: MODEL_EXTENSIONS },
        { name: $t("city3d.import.filter_images") as string, extensions: IMAGE_EXTENSIONS },
      ],
    });
    if (typeof p === "string") {
      path = p;
      error = null;
      const e = p.split(".").pop()?.toLowerCase() ?? "";
      if (e === "vrm") {
        klass = "character";
        licenseKind = "vrm";
      } else if (licenseKind === "vrm") {
        licenseKind = "own";
      }
    }
  }

  // Optional providers: Tripo generation (user's key) and local auto-rig.
  let providers = $state<ProvidersStatus | null>(null);
  let tripoKey = $state("");
  let genPrompt = $state("");
  let genClass = $state<AssetClass>("prop");
  let rigCli = $state("");
  let rigProgress = $state<string | null>(null);
  let cliProgress = $state<string | null>(null);
  let rigging = $state<string | null>(null);

  async function refreshProviders() {
    providers = await providersStatus().catch(() => null);
  }

  async function saveTripoKey() {
    try {
      await setTripoKey(tripoKey);
      tripoKey = "";
      await refreshProviders();
    } catch (e) {
      error = String(e);
    }
  }

  async function runGenerate() {
    if (!genPrompt.trim()) return;
    busy = true;
    error = null;
    try {
      const meta = await generate3d({ prompt: genPrompt.trim(), class: genClass, fitStyle });
      last = meta;
      genPrompt = "";
      await refresh();
      await show(meta.id);
      onimported?.(meta);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function saveRigCli() {
    try {
      await autorigConfigure(rigCli.trim() || null, null);
      await refreshProviders();
    } catch (e) {
      error = String(e);
    }
  }

  async function installCli() {
    cliProgress = "…";
    try {
      await autorigInstallCli((p) => {
        cliProgress = p.total ? `${Math.round((p.done / p.total) * 100)}%` : "…";
      });
      await refreshProviders();
    } catch (e) {
      error = String(e);
    } finally {
      cliProgress = null;
    }
  }

  async function installWeights() {
    rigProgress = "…";
    try {
      await autorigInstallWeights((p) => {
        rigProgress = p.total ? `${p.file} ${Math.round((p.done / p.total) * 100)}%` : p.file;
      });
      await refreshProviders();
    } catch (e) {
      error = String(e);
    } finally {
      rigProgress = null;
    }
  }

  async function runAutorig(a: AssetMeta) {
    rigging = a.id;
    error = null;
    try {
      const meta = await autorig(a.id);
      await refresh();
      await show(meta.id);
      onimported?.(meta);
    } catch (e) {
      error = String(e);
    } finally {
      rigging = null;
    }
  }

  async function refresh() {
    try {
      assets = await listAssets();
      for (const a of assets) {
        if (!thumbs[a.id]) {
          assetThumbUrl(a.id)
            .then((u) => (thumbs = { ...thumbs, [a.id]: u }))
            .catch(() => {});
        }
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function show(id: string) {
    selected = id;
    try {
      const gltf = await loadAsset(id);
      preview?.show(gltf.scene);
    } catch (e) {
      error = String(e);
    }
  }

  async function runImport() {
    if (!path) return;
    busy = true;
    error = null;
    try {
      const meta = await importFile(path, {
        class: klass,
        license: {
          kind: licenseKind,
          author: author.trim() || null,
          url: licenseUrl.trim() || null,
        },
        fitStyle,
        imageMode: isImage ? imageMode : undefined,
        palette,
      });
      last = meta;
      await refresh();
      await show(meta.id);
      onimported?.(meta);
      showToast("success", $t("city3d.import.done", { name: meta.name }) as string);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function remove(a: AssetMeta) {
    if (!confirm($t("city3d.import.remove_confirm", { name: a.name }) as string)) return;
    try {
      await deleteAsset(a.id);
      if (thumbs[a.id]) URL.revokeObjectURL(thumbs[a.id]);
      if (selected === a.id) {
        selected = null;
        preview?.show(null);
      }
      if (last?.id === a.id) last = null;
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  async function exportGlb(a: AssetMeta) {
    try {
      const p = await saveAssetGlb(a.id, a.name);
      if (p) showToast("success", $t("city3d.import.exported", { path: p }) as string);
    } catch (e) {
      error = String(e);
    }
  }

  async function exportVox(a: AssetMeta) {
    try {
      const p = await saveAssetVox(a.id, a.name, voxelMm);
      if (p) showToast("success", $t("city3d.import.exported", { path: p }) as string);
    } catch (e) {
      error = String(e);
    }
  }

  function permissionLine(a: AssetMeta): string {
    const p = a.vrm?.permissions;
    if (!p) return "";
    const yes = (b: boolean) => ($t(b ? "city3d.import.yes" : "city3d.import.no") as string);
    return $t("city3d.import.vrm_permissions", {
      avatar: yes(p.use_as_avatar),
      share: yes(p.redistribute_to_others),
      modify: yes(p.modify),
      export: yes(p.export),
    }) as string;
  }

  onMount(() => {
    void refreshProviders();
    if (canvas) {
      preview = createPreview(canvas);
      ro = new ResizeObserver(() => preview?.resize());
      ro.observe(canvas);
    }
    refresh();
  });

  onDestroy(() => {
    ro?.disconnect();
    preview?.dispose();
    for (const u of Object.values(thumbs)) URL.revokeObjectURL(u);
  });
</script>

<div class="import">
  <section class="form">
    <div class="row">
      <button type="button" class="button" onclick={pick} disabled={busy}>{$t("city3d.import.pick")}</button>
      <span class="file" title={path ?? ""}>{fileName || $t("city3d.import.no_file")}</span>
    </div>

    <label class="field">
      <span class="field-label">{$t("city3d.import.class")}</span>
      <select class="input" bind:value={klass} disabled={busy || isVrm}>
        {#each ASSET_CLASSES as c (c)}<option value={c}>{$t(`city3d.import.class_${c}`)}</option>{/each}
      </select>
    </label>

    {#if isImage}
      <label class="field">
        <span class="field-label">{$t("city3d.import.image_mode")}</span>
        <select class="input" bind:value={imageMode} disabled={busy}>
          {#each IMAGE_MODES as m (m)}<option value={m}>{$t(`city3d.import.mode_${m}`)}</option>{/each}
        </select>
      </label>
    {/if}

    <label class="field">
      <span class="field-label">{$t("city3d.import.license")}</span>
      <select class="input" bind:value={licenseKind} disabled={busy}>
        {#each LICENSE_KINDS as k (k)}
          <option value={k} disabled={k === "vrm" && !isVrm}>{$t(`city3d.import.license_${k.replace("-", "_")}`)}</option>
        {/each}
      </select>
    </label>
    {#if licenseKind === "cc-by" || licenseKind === "cc0"}
      <div class="pair">
        <label class="field">
          <span class="field-label">{$t("city3d.import.author")}{needsAuthor ? " *" : ""}</span>
          <input class="input" bind:value={author} disabled={busy} />
        </label>
        <label class="field">
          <span class="field-label">{$t("city3d.import.source_url")}</span>
          <input class="input" bind:value={licenseUrl} disabled={busy} placeholder="https://" />
        </label>
      </div>
    {/if}

    <label class="check">
      <input type="checkbox" bind:checked={fitStyle} disabled={busy} />
      <span>{$t("city3d.import.fit_style")}</span>
    </label>
    <p class="note">{$t("city3d.import.fit_style_hint")}</p>

    <div class="actions">
      <button type="button" class="button primary" disabled={!canImport} onclick={runImport}>
        {busy ? $t("city3d.import.working") : $t("city3d.import.import")}
      </button>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </section>

  <section class="preview-wrap">
    <canvas bind:this={canvas} class="preview" aria-label={$t("city3d.import.preview") as string}></canvas>
    {#if last}
      <div class="result">
        <p class="field-label">{last.name} · {$t(`city3d.import.class_${last.class}`)}</p>
        <p class="note">
          {$t("city3d.import.result", {
            before: kb(last.report.input.bytes),
            after: kb(last.report.output.bytes),
            tris_before: last.report.input.triangles,
            tris_after: last.report.output.triangles,
            ms: Math.round(last.report.total_ms),
          })}
        </p>
        <p class="note">
          {$t("city3d.import.footprint", {
            width: last.footprint.w_tiles,
            depth: last.footprint.d_tiles,
            height: (last.footprint.size_mm[1] / 1000).toFixed(2),
            anchors: last.anchors.length,
          })}
        </p>
        {#if last.vrm}<p class="note">{permissionLine(last)}</p>{/if}
        {#each last.report.warnings as w, i (i)}<p class="warn">{w}</p>{/each}
      </div>
    {/if}
  </section>

  <details class="generate">
    <summary class="field-label">{$t("city3d.import.gen_title")}</summary>
    <p class="note">{$t("city3d.import.gen_help")}</p>
    {#if providers?.tripo_key}
      <label class="field">
        <span class="field-label">{$t("city3d.import.gen_prompt")}</span>
        <textarea class="input" rows="2" bind:value={genPrompt}></textarea>
      </label>
      <div class="row">
        <select class="input" bind:value={genClass} aria-label={$t("city3d.import.class") as string}>
          {#each ASSET_CLASSES as c}<option value={c}>{$t(`city3d.import.class_${c}`)}</option>{/each}
        </select>
        <button type="button" class="button" onclick={runGenerate} disabled={busy || !genPrompt.trim()}>
          {busy ? $t("city3d.import.gen_running") : $t("city3d.import.gen_run")}
        </button>
      </div>
      <button type="button" class="button small" onclick={() => { tripoKey = ""; void setTripoKey("").then(refreshProviders); }}>{$t("city3d.import.gen_forget_key")}</button>
    {:else}
      <div class="row">
        <input class="input" type="password" placeholder={$t("city3d.import.gen_key") as string} bind:value={tripoKey} />
        <button type="button" class="button small" onclick={saveTripoKey} disabled={!tripoKey.trim()}>{$t("city3d.import.gen_key_save")}</button>
      </div>
    {/if}

    <p class="field-label">{$t("city3d.import.rig_title")}</p>
    <p class="note">{$t("city3d.import.rig_help")}</p>
    {#if !providers?.rig_cli}
      <div class="row">
        <input class="input" placeholder={$t("city3d.import.rig_cli") as string} bind:value={rigCli} />
        <button type="button" class="button small" onclick={saveRigCli} disabled={!rigCli.trim()}>{$t("city3d.import.rig_cli_save")}</button>
        <button
          type="button"
          class="button small"
          onclick={installCli}
          disabled={cliProgress !== null || !providers?.rig_cli_package}
          title={providers?.rig_cli_package ? undefined : ($t("city3d.import.rig_cli_install_unavailable") as string)}
        >
          {cliProgress ?? $t("city3d.import.rig_cli_install")}
        </button>
      </div>
    {/if}
    {#if !providers?.rig_weights}
      <button type="button" class="button small" onclick={installWeights} disabled={rigProgress !== null}>
        {rigProgress ?? $t("city3d.import.rig_weights")}
      </button>
    {/if}
    {#if providers?.rig_cli && providers?.rig_weights}<p class="note">{$t("city3d.import.rig_ready")}</p>{/if}
  </details>

  <section class="library">
    <div class="row">
      <p class="field-label">{$t("city3d.import.library", { count: assets.length })}</p>
      <label class="voxel">
        <span>{$t("city3d.import.voxel_mm")}</span>
        <input class="input" type="number" min="25" max="1000" step="25" bind:value={voxelMm} />
      </label>
    </div>
    {#if !assets.length}
      <p class="note">{$t("city3d.import.empty")}</p>
    {:else}
      <ul class="assets">
        {#each assets as a (a.id)}
          <li class:active={selected === a.id}>
            <button type="button" class="thumb" onclick={() => show(a.id)} aria-label={$t("city3d.import.show", { name: a.name }) as string}>
              {#if thumbs[a.id]}<img src={thumbs[a.id]} alt="" />{/if}
            </button>
            <div class="info">
              <span class="name">{a.name}</span>
              <span class="note">
                {$t(`city3d.import.class_${a.class}`)} · {kb(a.bytes)} · {$t("city3d.import.tris", { count: a.triangles })}
                · {$t(`city3d.import.license_${a.license.kind.replace("-", "_")}`)}
              </span>
            </div>
            <div class="buttons">
              <button type="button" class="button small" onclick={() => exportGlb(a)} disabled={!!a.vrm && !a.vrm.permissions.export}>GLB</button>
              <button type="button" class="button small" onclick={() => exportVox(a)}>.vox</button>
              {#if a.class === "character" && !a.vrm && a.source.kind !== "autorig" && providers?.rig_cli && providers?.rig_weights}
                <button type="button" class="button small" onclick={() => runAutorig(a)} disabled={rigging !== null}>
                  {rigging === a.id ? $t("city3d.import.rigging") : $t("city3d.import.autorig")}
                </button>
              {/if}
              <button type="button" class="button small" onclick={() => remove(a)}>{$t("city3d.import.remove")}</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<style>
  .generate {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  .generate summary {
    cursor: pointer;
  }
  .import {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 0.75rem);
    font-size: var(--text-sm, 0.875rem);
    min-width: 0;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    justify-content: space-between;
    min-width: 0;
  }
  .file {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary, inherit);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
    min-width: 0;
  }
  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2, 0.5rem);
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    cursor: pointer;
  }
  .note {
    margin: 0;
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
    font-size: var(--text-caption, 0.8rem);
  }
  .warn {
    margin: 0;
    color: var(--warning, #b25000);
    font-size: var(--text-caption, 0.8rem);
  }
  .error {
    margin: 0;
    color: var(--danger, #d70015);
  }
  .actions {
    display: flex;
    gap: var(--space-2, 0.5rem);
  }
  .preview-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  .preview {
    width: 100%;
    height: 220px;
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
    background: var(--surface-2, rgba(127, 127, 127, 0.08));
    display: block;
  }
  .result p {
    margin: 0;
  }
  .library {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }
  .voxel {
    display: flex;
    align-items: center;
    gap: var(--space-1, 0.25rem);
    font-size: var(--text-caption, 0.8rem);
    color: var(--text-dim, rgba(127, 127, 127, 0.9));
  }
  .voxel input {
    width: 5rem;
  }
  .assets {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
    max-height: 18rem;
    overflow: auto;
  }
  .assets li {
    display: grid;
    grid-template-columns: 48px 1fr auto;
    gap: var(--space-2, 0.5rem);
    align-items: center;
    padding: var(--space-1, 0.25rem);
    border-radius: var(--radius-md, 8px);
    border: 1px solid transparent;
  }
  .assets li.active {
    border-color: var(--accent, #0a84ff);
  }
  .thumb {
    width: 48px;
    height: 48px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm, 6px);
    background: var(--surface-2, rgba(127, 127, 127, 0.08));
    cursor: pointer;
    overflow: hidden;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .buttons {
    display: flex;
    gap: var(--space-1, 0.25rem);
  }
  .button.small {
    padding: 2px 8px;
    font-size: var(--text-caption, 0.8rem);
  }
</style>
