<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/photos/api";
  import { ASPECTS, EMPTY_EDITS, TONE_KEYS, cleanCrop, dateTitle, fitAspect, formatBytes, isIdentity, photoTime, rotate, sameEdits, type ToneKey } from "$lib/photos/plan";
  import type { Crop, Edits, Photo, Preview } from "$lib/photos/types";
  import CropBox from "./CropBox.svelte";

  let {
    photo,
    hasPrev,
    hasNext,
    onprev,
    onnext,
    onclose,
    onsaved,
    onexport,
  }: {
    photo: Photo;
    hasPrev: boolean;
    hasNext: boolean;
    onprev: () => void;
    onnext: () => void;
    onclose: () => void;
    onsaved: (path: string, edited: boolean) => void;
    onexport: (path: string) => void;
  } = $props();

  const SAVE_DELAY = 400;
  const FULL: Crop = { x: 0, y: 0, w: 1, h: 1 };

  let draft = $state<Edits>({ ...EMPTY_EDITS });
  let saved = $state<Edits>({ ...EMPTY_EDITS });
  let loadedFor = $state<string | null>(null);
  let view = $state<Preview | null>(null);
  let original = $state<Preview | null>(null);
  let comparing = $state(false);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let cropping = $state(false);
  let cropDraft = $state<Crop>({ ...FULL });
  let aspect = $state("free");
  let stageW = $state(0);
  let stageH = $state(0);

  let busy = false;
  let queued: { path: string; edits: Edits } | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  const tone: { key: ToneKey; label: string }[] = [
    { key: "exposure", label: "photos.editor.exposure" },
    { key: "contrast", label: "photos.editor.contrast" },
    { key: "saturation", label: "photos.editor.saturation" },
    { key: "temperature", label: "photos.editor.temperature" },
  ];

  let shown = $derived(comparing && original ? original : view);
  let fit = $derived.by(() => {
    if (!shown || stageW <= 0 || stageH <= 0) return { w: 0, h: 0 };
    const s = Math.min(stageW / shown.width, stageH / shown.height, 1.5);
    return { w: Math.round(shown.width * s), h: Math.round(shown.height * s) };
  });
  let imageAspect = $derived(view ? view.width / view.height : 1);
  let ratio = $derived.by(() => {
    const a = ASPECTS.find((x) => x.id === aspect);
    if (!a || a.ratio === null) return null;
    return a.ratio === 0 ? imageAspect : a.ratio;
  });
  let changed = $derived(!isIdentity(draft));
  let when = $derived(dateTitle(photoTime(photo), "day", $locale || "en"));

  function request(edits: Edits) {
    const path = photo.path;
    if (busy) {
      queued = { path, edits };
      return;
    }
    busy = true;
    void api
      .preview(path, edits)
      .then((p) => {
        if (path === photo.path) {
          view = p;
          error = null;
        }
      })
      .catch((e) => {
        if (path === photo.path) error = api.errorText(e);
      })
      .finally(() => {
        busy = false;
        if (path === photo.path) loading = false;
        const next = queued;
        queued = null;
        if (next && next.path === photo.path) request(next.edits);
      });
  }

  function render() {
    request(cropping ? { ...draft, crop: null } : draft);
  }

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    const path = photo.path;
    const edits = $state.snapshot(draft) as Edits;
    saveTimer = setTimeout(() => void persist(path, edits), SAVE_DELAY);
  }

  async function persist(path: string, edits: Edits) {
    saveTimer = null;
    if (sameEdits(edits, saved) && path === loadedFor) return;
    try {
      await api.editsSet(path, isIdentity(edits) ? null : edits);
      if (path === photo.path) saved = edits;
      onsaved(path, !isIdentity(edits));
    } catch (e) {
      error = api.errorText(e);
    }
  }

  function flushSave() {
    if (!saveTimer || !loadedFor) return;
    clearTimeout(saveTimer);
    void persist(loadedFor, $state.snapshot(draft) as Edits);
  }

  function update(next: Edits) {
    draft = next;
    render();
    scheduleSave();
  }

  function setTone(key: ToneKey, value: number) {
    update({ ...draft, [key]: value });
  }

  function turn(clockwise: boolean) {
    if (cropping) cropDraft = rotate({ ...draft, crop: cropDraft }, clockwise).crop ?? { ...FULL };
    update(rotate(draft, clockwise));
  }

  function startCrop() {
    cropDraft = draft.crop ? { ...draft.crop } : { ...FULL };
    aspect = "free";
    cropping = true;
    render();
  }

  function pickAspect(id: string) {
    aspect = id;
    const a = ASPECTS.find((x) => x.id === id);
    if (a && a.ratio !== null) cropDraft = fitAspect(cropDraft, a.ratio === 0 ? imageAspect : a.ratio, imageAspect);
  }

  function applyCrop() {
    cropping = false;
    update({ ...draft, crop: cleanCrop(cropDraft) });
  }

  function cancelCrop() {
    cropping = false;
    render();
  }

  function resetAll() {
    cropping = false;
    update({ ...EMPTY_EDITS });
  }

  async function load(path: string) {
    flushSave();
    loading = true;
    error = null;
    view = null;
    original = null;
    cropping = false;
    loadedFor = null;
    try {
      const stored = (await api.editsGet(path)) ?? { ...EMPTY_EDITS };
      if (path !== photo.path) return;
      saved = stored;
      draft = { ...stored };
      loadedFor = path;
      request(draft);
      if (!isIdentity(stored)) {
        void api
          .preview(path, { ...EMPTY_EDITS })
          .then((p) => {
            if (path === photo.path) original = p;
          })
          .catch(() => {});
      }
    } catch (e) {
      if (path === photo.path) {
        error = api.errorText(e);
        loading = false;
      }
    }
  }

  async function compareStart() {
    if (!original) {
      const path = photo.path;
      try {
        const p = await api.preview(path, { ...EMPTY_EDITS });
        if (path === photo.path) original = p;
      } catch {
        return;
      }
    }
    comparing = true;
  }

  $effect(() => {
    const path = photo.path;
    untrack(() => void load(path));
  });

  onMount(() => {
    const keydown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, select, textarea, [role='slider']")) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (cropping) cancelCrop();
        else onclose();
      } else if (e.key === "ArrowLeft" && hasPrev && !cropping) {
        e.preventDefault();
        onprev();
      } else if (e.key === "ArrowRight" && hasNext && !cropping) {
        e.preventDefault();
        onnext();
      } else if (e.key === "Enter" && cropping) {
        e.preventDefault();
        applyCrop();
      } else if (e.key.toLowerCase() === "r" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        turn(!e.shiftKey);
      }
    };
    window.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("keydown", keydown);
      flushSave();
      void api.release().catch(() => {});
    };
  });
</script>

<div class="ed" role="dialog" aria-modal="true" aria-label={photo.name}>
  <header class="ed-top">
    <button type="button" class="btn btn-ghost btn-sm" onclick={onclose}>{$t("photos.editor.back")}</button>
    <div class="ed-title">
      <strong>{photo.name}</strong>
      <span>{when} · {formatBytes(photo.size)}{#if view} · {view.full_width} × {view.full_height}{/if}</span>
    </div>
    <div class="ed-nav">
      <button type="button" class="btn btn-ghost btn-sm" disabled={!hasPrev || cropping} onclick={onprev} aria-label={$t("photos.editor.previous") as string}>‹</button>
      <button type="button" class="btn btn-ghost btn-sm" disabled={!hasNext || cropping} onclick={onnext} aria-label={$t("photos.editor.next") as string}>›</button>
    </div>
  </header>

  <div class="ed-body">
    <div class="ed-stage" bind:clientWidth={stageW} bind:clientHeight={stageH}>
      {#if error && !view}
        <div class="ed-msg" role="alert">
          <p>{$t("photos.editor.open_failed")}</p>
          <p class="ed-msg-detail">{error}</p>
        </div>
      {:else if shown}
        <div class="ed-frame" style:width="{fit.w}px" style:height="{fit.h}px">
          <img src={shown.src} alt={photo.name} draggable="false" />
          {#if cropping && !comparing}
            <CropBox crop={cropDraft} {ratio} {imageAspect} onchange={(c) => (cropDraft = c)} />
          {/if}
        </div>
      {/if}
      {#if loading}
        <span class="spinner ed-spin" aria-label={$t("photos.editor.loading") as string}></span>
      {/if}
    </div>

    <aside class="ed-side">
      {#if cropping}
        <section class="ed-group">
          <h3>{$t("photos.editor.crop")}</h3>
          <div class="ed-aspects" role="radiogroup" aria-label={$t("photos.editor.aspect") as string}>
            {#each ASPECTS as a (a.id)}
              <button type="button" class="btn btn-sm" class:btn-tinted={aspect === a.id} class:btn-ghost={aspect !== a.id} role="radio" aria-checked={aspect === a.id} onclick={() => pickAspect(a.id)}>
                {a.id === "free" ? $t("photos.editor.aspect_free") : a.id === "original" ? $t("photos.editor.aspect_original") : a.id}
              </button>
            {/each}
          </div>
          <p class="field-hint">{$t("photos.editor.crop_hint")}</p>
          <div class="ed-row">
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (cropDraft = { ...FULL })}>{$t("photos.editor.crop_clear")}</button>
            <span class="ed-grow"></span>
            <button type="button" class="btn btn-secondary btn-sm" onclick={cancelCrop}>{$t("photos.editor.cancel")}</button>
            <button type="button" class="btn btn-primary btn-sm" onclick={applyCrop}>{$t("photos.editor.crop_apply")}</button>
          </div>
        </section>
      {:else}
        <section class="ed-group">
          <h3>{$t("photos.editor.light_color")}</h3>
          {#each tone as item (item.key)}
            <label class="ed-slider">
              <span class="ed-slider-head">
                <span>{$t(item.label)}</span>
                <output>{draft[item.key] > 0 ? `+${draft[item.key]}` : draft[item.key]}</output>
              </span>
              <input
                type="range"
                min="-100"
                max="100"
                step="1"
                value={draft[item.key]}
                class:warm={item.key === "temperature"}
                oninput={(e) => setTone(item.key, Number(e.currentTarget.value))}
                ondblclick={() => setTone(item.key, 0)}
              />
            </label>
          {/each}
          <p class="field-hint">{$t("photos.editor.reset_hint")}</p>
        </section>
      {/if}

      <section class="ed-group">
        <h3>{$t("photos.editor.frame")}</h3>
        <div class="ed-row">
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => turn(false)}>{$t("photos.editor.rotate_left")}</button>
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => turn(true)}>{$t("photos.editor.rotate_right")}</button>
          {#if !cropping}
            <button type="button" class="btn btn-secondary btn-sm" disabled={!view} onclick={startCrop}>{draft.crop ? $t("photos.editor.crop_edit") : $t("photos.editor.crop")}</button>
          {/if}
        </div>
      </section>

      {#if error && view}
        <p class="field-error" role="alert">{error}</p>
      {/if}

      <footer class="ed-actions">
        <button
          type="button"
          class="btn btn-ghost btn-sm"
          disabled={!changed && !original}
          onpointerdown={() => void compareStart()}
          onpointerup={() => (comparing = false)}
          onpointerleave={() => (comparing = false)}
          onkeydown={(e) => {
            if (e.key === " " && !e.repeat) void compareStart();
          }}
          onkeyup={(e) => {
            if (e.key === " ") comparing = false;
          }}>{$t("photos.editor.compare")}</button
        >
        <button type="button" class="btn btn-ghost btn-sm" disabled={!changed} onclick={resetAll}>{$t("photos.editor.reset")}</button>
        <span class="ed-grow"></span>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => void api.reveal(photo.path).catch(() => {})}>{$t("photos.show_in_folder")}</button>
        <button
          type="button"
          class="btn btn-primary btn-sm"
          onclick={() => {
            flushSave();
            onexport(photo.path);
          }}>{$t("photos.export.action")}</button
        >
      </footer>
      <p class="field-hint">{$t("photos.editor.non_destructive")}</p>
    </aside>
  </div>
</div>

<style>
  .ed {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: var(--pane-bg);
  }

  .ed-top {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    padding-top: max(var(--space-3), env(titlebar-area-height, 0px));
    border-bottom: var(--hairline) solid var(--content-border);
  }

  .ed-title {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    text-align: center;
  }

  .ed-title strong {
    overflow: hidden;
    color: var(--text);
    font-size: var(--text-base);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ed-title span {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .ed-nav {
    display: flex;
    gap: var(--space-1);
  }

  .ed-body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .ed-stage {
    position: relative;
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    min-width: 0;
    margin: var(--space-4);
  }

  .ed-frame {
    position: relative;
    flex: none;
  }

  .ed-frame img {
    display: block;
    width: 100%;
    height: 100%;
    user-select: none;
    border-radius: var(--radius-sm);
  }

  .ed-spin {
    position: absolute;
    top: var(--space-3);
    right: var(--space-3);
  }

  .ed-msg {
    max-width: 420px;
    text-align: center;
    color: var(--text);
  }

  .ed-msg p {
    margin: 0 0 var(--space-2);
  }

  .ed-msg-detail {
    color: var(--text-dim);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .ed-side {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--space-4);
    width: 300px;
    padding: var(--space-4);
    overflow-y: auto;
    border-left: var(--hairline) solid var(--content-border);
    background: var(--surface);
  }

  .ed-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .ed-group h3 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 650;
  }

  .ed-slider {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .ed-slider-head {
    display: flex;
    justify-content: space-between;
  }

  .ed-slider output {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .ed-slider input {
    width: 100%;
    accent-color: var(--accent);
  }

  .ed-slider input.warm {
    accent-color: #e2a23b;
  }

  .ed-row,
  .ed-aspects {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ed-grow {
    flex: 1;
  }

  .ed-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: auto;
    padding-top: var(--space-3);
    border-top: var(--hairline) solid var(--content-border);
  }

  @media (max-width: 860px) {
    .ed-body {
      flex-direction: column;
    }

    .ed-side {
      width: auto;
      max-height: 45vh;
      border-left: none;
      border-top: var(--hairline) solid var(--content-border);
    }
  }
</style>
