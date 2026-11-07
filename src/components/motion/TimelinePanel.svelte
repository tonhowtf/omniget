<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/motion/api";
  import { PRESETS, findPreset, type Bezier } from "$lib/motion/bezier";
  import { num } from "$lib/motion/format";
  import { loop, reducedMotion } from "$lib/motion/reduced.svelte";
  import {
    LIMITS,
    MAX_DURATION,
    MIN_DURATION,
    TRACK_PROPS,
    emptyTracks,
    exportTracks,
    frameAt,
    isHex,
    keyCount,
    keyId,
    parseTimeline,
    safeName,
    sampleTimeline,
    serializeTimeline,
    sorted,
    toCssKeyframes,
    toWaapi,
    valueAt,
    type Keyframe,
    type StageKind,
    type Timeline,
    type TrackProp,
  } from "$lib/motion/timeline";
  import CodeBlock from "./CodeBlock.svelte";
  import CurveEditor from "./CurveEditor.svelte";

  let { active = true, incoming = null }: { active?: boolean; incoming?: { ease: Bezier; seq: number } | null } = $props();

  type FileState =
    | { kind: "idle" }
    | { kind: "busy"; label: string }
    | { kind: "ok"; message: string; path: string | null }
    | { kind: "error"; message: string };

  const reduced = reducedMotion();
  const SPEEDS = [0.25, 0.5, 1, 1.5, 2];

  let tl = $state<Timeline>(sampleTimeline());
  let time = $state(0);
  let playing = $state(false);
  let looping = $state(true);
  let speed = $state(1);
  let selected = $state<{ prop: TrackProp; id: string } | null>(null);
  let image = $state<api.StageImage | null>(null);
  let fileState = $state<FileState>({ kind: "idle" });
  let recent = $state<api.RecentTimeline[]>([]);
  let recentError = $state<string | null>(null);
  let exportName = $state("motion-box");
  let recentId = $state<string | null>(null);
  let imageDropped = $state(false);
  let dragging: { prop: TrackProp; id: string; lane: HTMLElement } | null = null;
  let lastIncoming = 0;

  let frame = $derived(frameAt(tl, time));
  let used = $derived(exportTracks(tl));
  let displayName = $derived(tl.name.trim() || ($t("motion.timeline.untitled") as string));
  let total = $derived(keyCount(tl));
  let selectedKey = $derived.by(() => {
    if (!selected) return null;
    const { prop, id } = selected;
    return tl.tracks[prop].find((k) => k.id === id) ?? null;
  });
  let selectedIsLast = $derived.by(() => {
    if (!selected || !selectedKey) return false;
    const keys = sorted(tl.tracks[selected.prop]);
    return keys[keys.length - 1]?.id === selectedKey.id;
  });
  let css = $derived(toCssKeyframes(tl, { name: exportName, iterations: looping ? "infinite" : 1, fill: !looping }));
  let waapi = $derived(toWaapi(tl, { iterations: looping ? "infinite" : 1, fill: !looping, selector: `.${safeName(exportName)}` }));

  let stageStyle = $derived.by(() => {
    const f = frame;
    const parts = [
      `transform: translate(${num(f.x, 2)}px, ${num(f.y, 2)}px) scale(${num(f.scale, 4)}) rotate(${num(f.rotate, 2)}deg)`,
      `opacity: ${num(Math.min(1, Math.max(0, f.opacity)), 4)}`,
    ];
    if (tl.stage.kind === "box") parts.push(`background-color: ${f.color}`);
    if (tl.stage.kind === "text") parts.push(`color: ${f.color}`);
    return parts.join("; ");
  });

  $effect(() => {
    if (!playing || !active) return;
    let last = performance.now();
    return loop((now) => {
      const dt = (now - last) * speed;
      last = now;
      let next = time + dt;
      if (next >= tl.duration) {
        if (looping) next = next % tl.duration;
        else {
          time = tl.duration;
          playing = false;
          return false;
        }
      }
      time = next;
    });
  });

  $effect(() => {
    const inc = incoming;
    if (!inc || inc.seq === lastIncoming) return;
    lastIncoming = inc.seq;
    untrack(() => {
      if (selectedKey) {
        selectedKey.ease = [...inc.ease];
        fileState = { kind: "ok", message: $t("motion.timeline.ease_applied") as string, path: null };
      } else {
        fileState = { kind: "error", message: $t("motion.timeline.ease_no_key") as string };
      }
    });
  });

  function togglePlay() {
    if (!playing && time >= tl.duration) time = 0;
    playing = !playing;
  }

  function stop() {
    playing = false;
    time = 0;
  }

  function setDuration(raw: string) {
    const n = Math.round(Number(raw));
    if (!Number.isFinite(n)) return;
    const next = Math.min(MAX_DURATION, Math.max(MIN_DURATION, n));
    const factor = next / tl.duration;
    for (const prop of TRACK_PROPS) for (const k of tl.tracks[prop]) k.time = Math.round(k.time * factor);
    tl.duration = next;
    time = Math.min(time, next);
  }

  function addKey(prop: TrackProp, at: number) {
    const atTime = Math.round(Math.min(tl.duration, Math.max(0, at)));
    const existing = tl.tracks[prop].find((k) => Math.abs(k.time - atTime) < 1);
    if (existing) {
      selected = { prop, id: existing.id };
      return;
    }
    const value = valueAt(prop, tl.tracks[prop], atTime);
    const before = sorted(tl.tracks[prop]).filter((k) => k.time < atTime).pop();
    const key: Keyframe = { id: keyId(), time: atTime, value, ease: before ? [...before.ease] : [0.42, 0, 0.58, 1] };
    tl.tracks[prop] = sorted([...tl.tracks[prop], key]);
    selected = { prop, id: key.id };
  }

  function removeKey(prop: TrackProp, id: string) {
    tl.tracks[prop] = tl.tracks[prop].filter((k) => k.id !== id);
    if (selected?.id === id) selected = null;
  }

  function laneTime(event: PointerEvent, lane: HTMLElement): number {
    const rect = lane.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (event.clientX - rect.left) / Math.max(1, rect.width)));
    return Math.round(f * tl.duration);
  }

  function laneDown(prop: TrackProp, event: PointerEvent) {
    if (event.button !== 0) return;
    const lane = event.currentTarget as HTMLElement;
    addKey(prop, laneTime(event, lane));
  }

  function keyDown(prop: TrackProp, key: Keyframe, event: PointerEvent) {
    event.stopPropagation();
    if (event.button !== 0) return;
    selected = { prop, id: key.id };
    const lane = (event.currentTarget as HTMLElement).parentElement;
    if (!lane) return;
    dragging = { prop, id: key.id, lane };
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  function keyMove(event: PointerEvent) {
    if (!dragging) return;
    const key = tl.tracks[dragging.prop].find((k) => k.id === dragging!.id);
    if (key) key.time = laneTime(event, dragging.lane);
  }

  function keyUp() {
    if (!dragging) return;
    const prop = dragging.prop;
    tl.tracks[prop] = sorted(tl.tracks[prop]);
    dragging = null;
  }

  function keyKey(prop: TrackProp, key: Keyframe, event: KeyboardEvent) {
    const step = event.shiftKey ? 100 : 10;
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      key.time = Math.min(tl.duration, Math.max(0, key.time + (event.key === "ArrowLeft" ? -step : step)));
      tl.tracks[prop] = sorted(tl.tracks[prop]);
      selected = { prop, id: key.id };
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      removeKey(prop, key.id);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      selected = { prop, id: key.id };
      time = key.time;
    }
  }

  function setKeyTime(raw: string) {
    if (!selected || !selectedKey) return;
    const n = Math.round(Number(raw));
    if (!Number.isFinite(n)) return;
    selectedKey.time = Math.min(tl.duration, Math.max(0, n));
    tl.tracks[selected.prop] = sorted(tl.tracks[selected.prop]);
  }

  function setKeyValue(raw: string) {
    if (!selected || !selectedKey) return;
    if (selected.prop === "color") {
      if (isHex(raw)) selectedKey.value = raw;
      return;
    }
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    const lim = LIMITS[selected.prop];
    selectedKey.value = Math.min(lim.max, Math.max(lim.min, n));
  }

  function setKeyPreset(id: string) {
    const preset = PRESETS.find((p) => p.id === id);
    if (preset && selectedKey) selectedKey.ease = [...preset.value];
  }

  function setStage(kind: StageKind) {
    imageDropped = false;
    tl.stage.kind = kind;
    if (kind !== "image") tl.stage.imageName = null;
    else if (!image) void chooseImage();
  }

  async function chooseImage() {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: $t("motion.timeline.image_filter") as string, extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg"] }],
    }).catch(() => null);
    if (typeof picked !== "string") {
      if (!image) tl.stage.kind = "box";
      return;
    }
    fileState = { kind: "busy", label: $t("motion.timeline.loading_image") as string };
    try {
      image = await api.openImage(picked);
      tl.stage.kind = "image";
      tl.stage.imageName = image.name;
      fileState = { kind: "idle" };
    } catch (e) {
      if (!image) tl.stage.kind = "box";
      fileState = { kind: "error", message: $t("motion.timeline.image_error", { error: api.errorText(e) }) as string };
    }
  }

  function clearAll() {
    tl = { version: 1, name: tl.name, duration: tl.duration, stage: tl.stage, tracks: emptyTracks() };
    selected = null;
    time = 0;
  }

  function loadSample() {
    tl = sampleTimeline();
    recentId = null;
    imageDropped = false;
    selected = null;
    time = 0;
  }

  function applyLoaded(text: string, source: string, id: string | null): boolean {
    const parsed = parseTimeline(text);
    if (!parsed.ok) {
      fileState = { kind: "error", message: $t(`motion.timeline.parse_${parsed.error}`, { name: source }) as string };
      return false;
    }
    tl = parsed.timeline;
    recentId = id;
    imageDropped = parsed.imageDropped;
    selected = null;
    time = 0;
    playing = false;
    exportName = safeName(tl.name);
    return true;
  }

  async function saveAs() {
    const path = await saveDialog({
      defaultPath: `${safeName(tl.name)}.json`,
      filters: [{ name: "JSON", extensions: ["json"] }],
    }).catch(() => null);
    if (!path) return;
    fileState = { kind: "busy", label: $t("motion.timeline.saving") as string };
    try {
      const saved = await api.saveTimeline(path, serializeTimeline(tl));
      fileState = { kind: "ok", message: $t("motion.timeline.saved", { name: api.fileName(saved.path) }) as string, path: saved.path };
    } catch (e) {
      fileState = { kind: "error", message: $t("motion.timeline.save_error", { error: api.errorText(e) }) as string };
    }
  }

  async function openFile() {
    const picked = await openDialog({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] }).catch(() => null);
    if (typeof picked !== "string") return;
    fileState = { kind: "busy", label: $t("motion.timeline.opening") as string };
    try {
      const text = await api.loadTimeline(picked);
      if (applyLoaded(text, api.fileName(picked), null)) fileState = { kind: "ok", message: $t("motion.timeline.opened", { name: api.fileName(picked) }) as string, path: null };
    } catch (e) {
      fileState = { kind: "error", message: $t("motion.timeline.open_error", { error: api.errorText(e) }) as string };
    }
  }

  async function saveRecent() {
    fileState = { kind: "busy", label: $t("motion.timeline.saving") as string };
    try {
      const name = displayName;
      recent = await api.recentSave(recentId, name, serializeTimeline(tl));
      recentId = recent[0]?.id ?? null;
      recentError = null;
      fileState = { kind: "ok", message: $t("motion.timeline.recent_saved", { name }) as string, path: null };
    } catch (e) {
      fileState = { kind: "error", message: $t("motion.timeline.save_error", { error: api.errorText(e) }) as string };
    }
  }

  function openRecent(entry: api.RecentTimeline) {
    if (applyLoaded(entry.timeline, entry.name, entry.id)) fileState = { kind: "ok", message: $t("motion.timeline.opened", { name: entry.name }) as string, path: null };
  }

  async function removeRecent(entry: api.RecentTimeline) {
    try {
      recent = await api.recentRemove(entry.id);
      if (recentId === entry.id) recentId = null;
    } catch (e) {
      recentError = api.errorText(e);
    }
  }

  function when(ms: number): string {
    try {
      return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    } catch {
      return "";
    }
  }

  function valueLabel(prop: TrackProp, value: number | string): string {
    if (prop === "color") return String(value);
    const unit = LIMITS[prop].unit;
    return `${num(Number(value), 2)}${unit}`;
  }

  onMount(() => {
    void api
      .recentList()
      .then((list) => (recent = list))
      .catch((e) => (recentError = api.errorText(e)));
  });
</script>

<div class="tp">
  <section class="mo-card" aria-labelledby="tp-stage-title">
    <div class="mo-card-head">
      <h2 id="tp-stage-title" class="mo-card-title">{$t("motion.timeline.stage")}</h2>
      <div class="segmented" role="radiogroup" aria-label={$t("motion.timeline.element")}>
        {#each ["box", "text", "image"] as const as kind (kind)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={tl.stage.kind === kind} onclick={() => setStage(kind)}>{$t(`motion.timeline.kind_${kind}`)}</button>
        {/each}
      </div>
      {#if tl.stage.kind === "text"}
        <input class="input-text tp-text" type="text" maxlength="80" bind:value={tl.stage.text} aria-label={$t("motion.timeline.text_label")} />
      {:else if tl.stage.kind === "image"}
        <button type="button" class="btn btn-secondary btn-sm" onclick={chooseImage}>{image ? $t("motion.timeline.change_image") : $t("motion.timeline.choose_image")}</button>
      {/if}
    </div>
    <div class="tp-stage" aria-hidden="true">
      <div class="tp-origin"></div>
      {#if tl.stage.kind === "box"}
        <div class="tp-el tp-box" style={stageStyle}></div>
      {:else if tl.stage.kind === "text"}
        <div class="tp-el tp-word" style={stageStyle}>{tl.stage.text || " "}</div>
      {:else if image}
        <img class="tp-el tp-img" style={stageStyle} src={image.data_url} alt="" draggable="false" />
      {/if}
    </div>
    {#if tl.stage.kind === "image" && image}
      <p class="mo-meta">{$t("motion.timeline.image_note", { name: image.name })}</p>
    {/if}
    {#if imageDropped}
      <p class="mo-note" role="note">{$t("motion.timeline.image_dropped")}</p>
    {/if}

    <div class="mo-transport">
      <button type="button" class="btn btn-primary btn-sm" onclick={togglePlay} aria-pressed={playing} disabled={total === 0} title={total === 0 ? ($t("motion.timeline.no_keys") as string) : undefined}>
        {playing ? $t("motion.pause") : $t("motion.play")}
      </button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={stop}>{$t("motion.timeline.rewind")}</button>
      <label class="mo-check"><input type="checkbox" bind:checked={looping} />{$t("motion.timeline.loop")}</label>
      <label class="tp-inline">
        {$t("motion.timeline.speed")}
        <select class="select" bind:value={speed}>
          {#each SPEEDS as s (s)}<option value={s}>{s}×</option>{/each}
        </select>
      </label>
      <label class="tp-inline">
        {$t("motion.duration")}
        <input class="input-number tp-num" type="number" min={MIN_DURATION} max={MAX_DURATION} step="100" value={tl.duration} onchange={(e) => setDuration((e.currentTarget as HTMLInputElement).value)} />
        {$t("motion.unit_ms")}
      </label>
      <span class="tp-clock" aria-live="off">{$t("motion.timeline.clock", { now: Math.round(time), total: tl.duration })}</span>
    </div>
    {#if reduced.current}
      <p class="mo-note" role="note">{$t("motion.timeline.reduced_note")}</p>
    {/if}
  </section>

  <section class="mo-card" aria-labelledby="tp-tracks-title">
    <div class="mo-card-head">
      <h2 id="tp-tracks-title" class="mo-card-title">{$t("motion.timeline.tracks")}</h2>
      <span class="mo-meta">{$t("motion.timeline.tracks_hint")}</span>
      <button type="button" class="btn btn-ghost btn-sm" onclick={loadSample}>{$t("motion.timeline.sample")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={clearAll} disabled={total === 0}>{$t("motion.timeline.clear")}</button>
    </div>
    <div class="tp-grid" onpointermove={keyMove} onpointerup={keyUp} onpointercancel={keyUp} role="presentation">
      <div class="tp-row tp-ruler">
        <span></span>
        <input
          class="tp-scrub"
          type="range"
          min="0"
          max={tl.duration}
          step="1"
          bind:value={time}
          oninput={() => (playing = false)}
          aria-label={$t("motion.timeline.scrub")}
          aria-valuetext={$t("motion.ms_value", { value: Math.round(time) }) as string}
        />
        <span></span>
      </div>
      {#each TRACK_PROPS as prop (prop)}
        <div class="tp-row">
          <span class="tp-name">
            {$t(`motion.prop.${prop}`)}
            <span class="tp-now">{valueLabel(prop, valueAt(prop, tl.tracks[prop], time))}</span>
          </span>
          <div class="tp-lane" class:empty={tl.tracks[prop].length === 0} onpointerdown={(e) => laneDown(prop, e)} role="presentation">
            <div class="tp-head" style:left="{(time / tl.duration) * 100}%"></div>
            {#each tl.tracks[prop] as key (key.id)}
              {@const isSel = selected?.id === key.id}
              <button
                type="button"
                class="tp-key"
                class:sel={isSel}
                style:left="{(key.time / tl.duration) * 100}%"
                aria-label={$t("motion.timeline.key_label", { prop: $t(`motion.prop.${prop}`), time: Math.round(key.time), value: valueLabel(prop, key.value) })}
                aria-pressed={isSel}
                onpointerdown={(e) => keyDown(prop, key, e)}
                onkeydown={(e) => keyKey(prop, key, e)}
                onclick={() => (selected = { prop, id: key.id })}
              ></button>
            {/each}
          </div>
          <button type="button" class="btn btn-ghost btn-sm tp-add" onclick={() => addKey(prop, time)} aria-label={$t("motion.timeline.add_key_label", { prop: $t(`motion.prop.${prop}`) })}>+</button>
        </div>
      {/each}
    </div>
    {#if total === 0}
      <p class="mo-note">{$t("motion.timeline.empty")}</p>
    {/if}
  </section>

  <section class="mo-card tp-inspector" aria-labelledby="tp-key-title">
    <h2 id="tp-key-title" class="mo-card-title">{$t("motion.timeline.keyframe")}</h2>
    {#if selected && selectedKey}
      {@const prop = selected.prop}
      <div class="tp-fields">
        <label class="tp-field">
          <span>{$t("motion.timeline.time")}</span>
          <input class="input-number" type="number" min="0" max={tl.duration} step="10" value={Math.round(selectedKey.time)} onchange={(e) => setKeyTime((e.currentTarget as HTMLInputElement).value)} />
        </label>
        <label class="tp-field">
          <span>{$t(`motion.prop.${prop}`)}</span>
          {#if prop === "color"}
            <input class="tp-color" type="color" value={String(selectedKey.value)} oninput={(e) => setKeyValue((e.currentTarget as HTMLInputElement).value)} />
          {:else}
            <input
              class="input-number"
              type="number"
              min={LIMITS[prop].min}
              max={LIMITS[prop].max}
              step={LIMITS[prop].step}
              value={Number(selectedKey.value)}
              onchange={(e) => setKeyValue((e.currentTarget as HTMLInputElement).value)}
            />
          {/if}
        </label>
        <label class="tp-field">
          <span>{$t("motion.timeline.ease")}</span>
          <select class="select" value={findPreset(selectedKey.ease)?.id ?? ""} onchange={(e) => setKeyPreset((e.currentTarget as HTMLSelectElement).value)} disabled={selectedIsLast}>
            <option value="" disabled>{$t("motion.easing.custom")}</option>
            {#each PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
          </select>
        </label>
      </div>
      {#if selectedIsLast}
        <p class="mo-meta">{$t("motion.timeline.last_key")}</p>
      {:else}
        <CurveEditor bind:value={selectedKey.ease} label={$t("motion.timeline.segment_curve")} compact />
      {/if}
      <div class="mo-transport">
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (time = selectedKey!.time)}>{$t("motion.timeline.go_to_key")}</button>
        <button type="button" class="btn btn-destructive btn-sm" onclick={() => removeKey(prop, selectedKey!.id)}>{$t("motion.timeline.delete_key")}</button>
      </div>
    {:else}
      <p class="mo-meta">{$t("motion.timeline.no_selection")}</p>
    {/if}
  </section>

  <section class="mo-card tp-files" aria-labelledby="tp-file-title">
    <div class="mo-card-head">
      <h2 id="tp-file-title" class="mo-card-title">{$t("motion.timeline.file")}</h2>
    </div>
    <label class="tp-field">
      <span>{$t("motion.timeline.name")}</span>
      <input class="input-text" type="text" maxlength="80" bind:value={tl.name} placeholder={$t("motion.timeline.untitled")} />
    </label>
    <div class="mo-transport">
      <button type="button" class="btn btn-secondary btn-sm" onclick={saveAs} disabled={fileState.kind === "busy"}>{$t("motion.timeline.save_as")}</button>
      <button type="button" class="btn btn-secondary btn-sm" onclick={openFile} disabled={fileState.kind === "busy"}>{$t("motion.timeline.open")}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={saveRecent} disabled={fileState.kind === "busy"}>{$t("motion.timeline.save_recent")}</button>
    </div>
    <div class="mo-status" role="status" aria-live="polite">
      {#if fileState.kind === "busy"}
        <span class="spinner mo-spin" aria-hidden="true"></span>{fileState.label}
      {:else if fileState.kind === "ok"}
        <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" class="mo-ok" aria-hidden="true"><circle cx="8" cy="8" r="6.5" /><path d="m5.2 8.2 1.9 1.9 3.7-4" /></svg>
        <span>{fileState.message}</span>
        {#if fileState.path}
          {@const p = fileState.path}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => void api.reveal(p).catch(() => {})}>{$t("motion.show_in_folder")}</button>
        {/if}
      {:else if fileState.kind === "error"}
        <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" class="mo-bad" aria-hidden="true"><circle cx="8" cy="8" r="6.5" /><path d="M8 4.8v3.8M8 11.1v.1" /></svg>
        <span>{fileState.message}</span>
      {/if}
    </div>
    <h3 class="mo-sub">{$t("motion.timeline.recent")}</h3>
    {#if recentError}
      <p class="mo-meta">{$t("motion.timeline.recent_error", { error: recentError })}</p>
    {:else if recent.length === 0}
      <p class="mo-meta">{$t("motion.timeline.recent_empty")}</p>
    {:else}
      <ul class="tp-recent">
        {#each recent as entry (entry.id)}
          <li>
            <button type="button" class="tp-recent-open" onclick={() => openRecent(entry)}>
              <span class="tp-recent-name">{entry.name}</span>
              <span class="tp-recent-when">{when(entry.saved_at_ms)}</span>
            </button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => removeRecent(entry)} aria-label={$t("motion.timeline.recent_remove", { name: entry.name })}>×</button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="mo-card tp-export" aria-labelledby="tp-export-title">
    <div class="mo-card-head">
      <h2 id="tp-export-title" class="mo-card-title">{$t("motion.export")}</h2>
      <label class="tp-inline">
        {$t("motion.timeline.class_name")}
        <input class="input-text" type="text" maxlength="40" bind:value={exportName} />
      </label>
    </div>
    {#if used.length === 0}
      <p class="mo-meta">{$t("motion.timeline.export_empty")}</p>
    {:else}
      <div class="mo-codes">
        <CodeBlock label="CSS @keyframes" code={css} hint={$t("motion.timeline.css_hint")} />
        <CodeBlock label="Web Animations API" code={waapi} hint={$t("motion.timeline.waapi_hint")} />
      </div>
    {/if}
  </section>
</div>

<style>
  .tp {
    display: grid;
    grid-template-columns: 1fr minmax(260px, 320px);
    gap: var(--space-4);
  }

  .tp > :nth-child(1),
  .tp > :nth-child(2),
  .tp-export {
    grid-column: 1 / -1;
  }

  .tp-text {
    width: 180px;
  }

  .tp-stage {
    position: relative;
    height: 260px;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border-radius: var(--radius-md);
    background:
      linear-gradient(var(--separator) 1px, transparent 1px) 0 0 / 40px 40px,
      linear-gradient(90deg, var(--separator) 1px, transparent 1px) 0 0 / 40px 40px,
      var(--fill-1);
  }

  .tp-origin {
    position: absolute;
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    box-shadow: 0 0 0 1px var(--text-faint);
  }

  .tp-el {
    position: absolute;
    will-change: transform, opacity;
  }

  .tp-box {
    width: 64px;
    height: 64px;
    border-radius: var(--radius-lg);
  }

  .tp-word {
    font-family: var(--font-display);
    font-size: var(--text-3xl);
    font-weight: 700;
    white-space: nowrap;
  }

  .tp-img {
    max-width: 120px;
    max-height: 120px;
    user-select: none;
  }

  .tp-inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .tp-num {
    width: 80px;
  }

  .tp-clock {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .tp-grid {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tp-row {
    display: grid;
    grid-template-columns: 120px 1fr 32px;
    align-items: center;
    gap: var(--space-2);
  }

  .tp-scrub {
    width: 100%;
    margin: 0;
    accent-color: var(--accent);
  }

  .tp-name {
    display: flex;
    flex-direction: column;
    font-size: var(--text-sm);
    color: var(--text);
  }

  .tp-now {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }

  .tp-lane {
    position: relative;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    cursor: copy;
  }

  .tp-lane.empty {
    background: repeating-linear-gradient(90deg, var(--fill-1) 0 6px, transparent 6px 12px);
  }

  .tp-head {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--accent);
    pointer-events: none;
  }

  .tp-key {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    padding: 0;
    border: 2px solid var(--accent);
    border-radius: 2px;
    background: var(--surface-hi);
    transform: rotate(45deg);
    cursor: ew-resize;
  }

  .tp-key.sel {
    background: var(--accent);
  }

  .tp-key:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .tp-add {
    padding: 0;
    width: 28px;
  }

  .tp-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  .tp-fields .tp-field:last-child {
    grid-column: 1 / -1;
  }

  .tp-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .tp-field input,
  .tp-field select {
    width: 100%;
    max-width: none;
  }

  .tp-color {
    height: var(--control-h);
    padding: 0;
    border: none;
    background: none;
  }

  .tp-recent {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tp-recent li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .tp-recent-open {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .tp-recent-open:hover {
    background: var(--fill-1);
  }

  .tp-recent-open:focus-visible {
    outline: var(--focus-ring);
  }

  .tp-recent-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tp-recent-when {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  @media (max-width: 760px) {
    .tp {
      grid-template-columns: 1fr;
    }
  }
</style>
