<script lang="ts">
  /**
   * Avatar creator: edits a genome (body phenotypes, face changes, skin, hair,
   * clothing, colors, gait and mood) with a live animated 3D preview, saves it
   * through the avatar commands and exports a VRM 1.0 file.
   * `mode: 'self'` edits the player's avatar (city3d_avatar_get/set);
   * `mode: 'npc'` edits the given genome and hands it back through `onSave`.
   */
  import { onDestroy, onMount } from "svelte";
  import { save } from "@tauri-apps/plugin-dialog";
  import * as THREE from "three";
  import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { loadAnny, type AnnyBase } from "$lib/city3d/avatar/anny";
  import { buildCharacter, type CharacterRig } from "$lib/city3d/avatar/assemble";
  import { Animator } from "$lib/city3d/avatar/animator";
  import { catalog } from "$lib/city3d/avatar/catalog";
  import { loadClips, type Clip } from "$lib/city3d/avatar/clips";
  import { MOOD_FACE, toArkit } from "$lib/city3d/avatar/expressions";
  import { decodeGenome, encodeGenome, fixGenome, validateGenome, NO_FACE, type Genome } from "$lib/city3d/avatar/genome";
  import { exportVrm } from "$lib/city3d/avatar/vrm-export";
  import * as api from "$lib/city3d/avatar/api";

  interface Props {
    /** default "self" */
    mode?: "self" | "npc";
    /** genome hex to start from (npc mode, or to override the saved avatar) */
    genome?: string;
    onSave?: (genome: string) => void;
  }
  let { mode = "self", genome: initial, onSave }: Props = $props();

  let g = $state<Genome | null>(null);
  let original = $state<string | null>(null);
  let seed = $state(Math.floor(Math.random() * 1e6));
  let busy = $state(false);
  let error = $state<string | null>(null);
  let previewClip = $state("idle");
  let turntable = $state(true);
  let vrmName = $state("");
  let vrmAuthor = $state("");
  let tab = $state<"body" | "face" | "look" | "clothes" | "manner">("body");

  let canvas: HTMLCanvasElement | undefined = $state();
  let host: HTMLDivElement | undefined = $state();
  let base: AnnyBase | null = null;
  let clips: Map<string, Clip> | null = null;
  let renderer: THREE.WebGLRenderer | null = null;
  let scene: THREE.Scene | null = null;
  let camera: THREE.PerspectiveCamera | null = null;
  let controls: OrbitControls | null = null;
  let rig: CharacterRig | null = null;
  let anim: Animator | null = null;
  let raf = 0;
  let resizeObs: ResizeObserver | null = null;
  let rebuildTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;

  const hex = $derived(g ? encodeGenome(g) : "");
  const problems = $derived(g ? validateGenome(g) : []);
  const dirty = $derived(!!g && hex !== original);

  const PHENO = ["gender", "age", "muscle", "weight", "height", "proportions"] as const;
  const PREVIEW = ["idle", "walk", "talk", "wave", "work", "sit", "cheer"];
  const pretty = (id: string) => id.replace(/-/g, " ");

  async function start() {
    try {
      [base, clips] = await Promise.all([loadAnny(), loadClips()]);
      let hexStart = initial ?? null;
      if (!hexStart && mode === "self") {
        const rec = await api.avatarGet().catch(() => null);
        hexStart = rec?.genome ?? null;
      }
      if (!hexStart) hexStart = (await api.genomeRandom(seed)).genome;
      original = hexStart;
      g = decodeGenome(hexStart);
      setupScene();
      rebuild();
    } catch (e) {
      error = String(e);
    }
  }

  function setupScene() {
    if (!canvas || !host) return;
    renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, preserveDrawingBuffer: true });
    renderer.setPixelRatio(Math.min(2, window.devicePixelRatio));
    scene = new THREE.Scene();
    camera = new THREE.PerspectiveCamera(32, 1, 0.05, 50);
    camera.position.set(0.8, 1.35, 4.4);
    scene.add(new THREE.HemisphereLight(0xffffff, 0x8a7f72, 1.4));
    const sun = new THREE.DirectionalLight(0xffffff, 2.0);
    sun.position.set(1.5, 3, 2.5);
    scene.add(sun);
    const floor = new THREE.Mesh(new THREE.CircleGeometry(0.9, 48), new THREE.MeshToonMaterial({ color: 0xcfd6cf }));
    floor.rotation.x = -Math.PI / 2;
    scene.add(floor);
    controls = new OrbitControls(camera, canvas);
    controls.target.set(0, 0.95, 0);
    controls.enableDamping = true;
    controls.minDistance = 0.6;
    controls.maxDistance = 6;
    controls.update();
    resizeObs = new ResizeObserver(resize);
    resizeObs.observe(host);
    resize();
    let last = performance.now();
    const loop = () => {
      raf = requestAnimationFrame(loop);
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      tick(dt);
    };
    loop();
  }

  function resize() {
    if (!host || !renderer || !camera) return;
    const w = host.clientWidth, h = host.clientHeight;
    renderer.setSize(w, h, false);
    camera.aspect = w / Math.max(1, h);
    camera.updateProjectionMatrix();
  }

  let clock = 0;
  function tick(dt: number) {
    if (!renderer || !scene || !camera) return;
    clock += dt;
    if (rig && anim) {
      anim.play(previewClip === "wave" || previewClip === "cheer" ? "idle" : previewClip);
      anim.step(dt);
      anim.apply(rig.bones, rig.hipsRest);
      if (turntable) rig.root.rotation.y += dt * 0.35;
      rig.root.updateMatrixWorld(true);
      const infl = rig.body.morphTargetInfluences;
      const dict = rig.body.morphTargetDictionary;
      if (infl && dict && g) {
        const w: Record<string, number> = { ...(MOOD_FACE[catalog.moods[g.mood]] ?? {}) };
        const b = (clock % 4) / 0.15;
        if (b < 1) w.blink = Math.sin(b * Math.PI);
        if (previewClip === "talk") w.aa = Math.max(0, Math.sin(clock * 9)) * 0.6;
        infl.fill(0);
        for (const [k, v] of Object.entries(toArkit(w))) if (dict[k] !== undefined) infl[dict[k]] = v;
      }
      if ((previewClip === "wave" || previewClip === "cheer") && !anim.gesture) {
        anim.gestureAction({ gesture: { mode: "clip", clip: previewClip } });
      }
    }
    controls?.update();
    renderer.render(scene, camera);
  }

  function rebuild() {
    if (rebuildTimer) clearTimeout(rebuildTimer);
    rebuildTimer = setTimeout(() => {
      if (!base || !clips || !scene || !g || disposed) return;
      const next = buildCharacter(base, fixGenome($state.snapshot(g) as Genome), "full");
      const yaw = rig?.root.rotation.y ?? 0;
      rig?.dispose();
      rig = next;
      rig.root.rotation.y = yaw;
      scene.add(rig.root);
      if (!anim) anim = new Animator(clips);
      if (controls) controls.target.set(0, rig.shape.height * 0.55, 0);
    }, 60);
  }

  function change(mut: (x: Genome) => void) {
    if (!g) return;
    mut(g);
    const fixed = fixGenome($state.snapshot(g) as Genome);
    g.top = fixed.top;
    g.bottom = fixed.bottom;
    rebuild();
  }

  async function randomize() {
    busy = true;
    try {
      g = decodeGenome((await api.genomeRandom(seed)).genome);
      rebuild();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function reset() {
    if (!original) return;
    g = decodeGenome(original);
    rebuild();
  }

  async function doSave() {
    if (!g || problems.length) return;
    busy = true;
    try {
      if (mode === "self") {
        const rec = await api.avatarSet({ genome: hex });
        original = rec.genome;
        showToast("success", $t("city3d.avatar.saved") as string);
      } else {
        original = hex;
      }
      onSave?.(hex);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  async function doExport() {
    if (!g || !base) return;
    const path = await save({
      defaultPath: `${(vrmName || "avatar").replace(/[^\w.-]+/g, "_")}.vrm`,
      filters: [{ name: "VRM", extensions: ["vrm"] }],
    });
    if (!path) return;
    busy = true;
    try {
      const res = await exportVrm(base, $state.snapshot(g) as Genome, {
        name: vrmName || ($t("city3d.avatar.defaultName") as string),
        authors: vrmAuthor ? [vrmAuthor] : [],
      });
      await api.exportVrmFile(path, toBase64(res.bytes));
      showToast("success", $t("city3d.avatar.exported") as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busy = false;
    }
  }

  function toBase64(buf: ArrayBuffer): string {
    const bytes = new Uint8Array(buf);
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  }

  const swatch = (h?: string) => `background:#${h ?? "888"}`;

  onMount(() => {
    start();
  });

  onDestroy(() => {
    disposed = true;
    cancelAnimationFrame(raf);
    if (rebuildTimer) clearTimeout(rebuildTimer);
    resizeObs?.disconnect();
    controls?.dispose();
    rig?.dispose();
    renderer?.dispose();
  });
</script>

<div class="ac-wrap">
  <div class="ac">
    <div class="ac-preview" bind:this={host}>
      <canvas bind:this={canvas}></canvas>
      <div class="ac-preview-bar">
        <select bind:value={previewClip} aria-label={$t("city3d.avatar.preview")}>
          {#each PREVIEW as c (c)}
            <option value={c}>{$t(`city3d.avatar.clip.${c}`)}</option>
          {/each}
        </select>
        <label class="ac-check"><input type="checkbox" bind:checked={turntable} /> {$t("city3d.avatar.turntable")}</label>
      </div>
      {#if error}<p class="ac-error">{error}</p>{/if}
    </div>

    {#if g}
      <div class="ac-panel">
        <div class="ac-tabs" role="tablist">
          {#each ["body", "face", "look", "clothes", "manner"] as const as k (k)}
            <button role="tab" aria-selected={tab === k} class:on={tab === k} onclick={() => (tab = k)}>{$t(`city3d.avatar.tab.${k}`)}</button>
          {/each}
        </div>

        <div class="ac-body">
          {#if tab === "body"}
            {#each PHENO as p, i (p)}
              <label class="ac-row">
                <span>{$t(`city3d.avatar.pheno.${p}`)}</span>
                <input type="range" min="0" max="255" value={g.phenotypes[i]}
                  oninput={(e) => change((x) => (x.phenotypes[i] = +e.currentTarget.value))} />
              </label>
            {/each}
          {:else if tab === "face"}
            {#each g.face as f, i (i)}
              <div class="ac-row">
                <select value={f[0]} onchange={(e) => change((x) => (x.face[i] = [+e.currentTarget.value, x.face[i][1] || 60]))}
                  aria-label={$t("city3d.avatar.faceTarget")}>
                  <option value={NO_FACE}>{$t("city3d.avatar.none")}</option>
                  {#each catalog.face_targets as ft, k (ft.id)}
                    <option value={k}>{pretty(ft.id)}</option>
                  {/each}
                </select>
                <input type="range" min="-127" max="127" value={f[1]} disabled={f[0] === NO_FACE}
                  aria-label={$t("city3d.avatar.amount")}
                  oninput={(e) => change((x) => (x.face[i] = [x.face[i][0], +e.currentTarget.value]))} />
              </div>
            {/each}
            <p class="ac-hint">{$t("city3d.avatar.faceHint")}</p>
          {:else if tab === "look"}
            <span class="ac-label">{$t("city3d.avatar.skin")}</span>
            <div class="ac-swatches">
              {#each catalog.skin_tones as s, k (s.id)}
                <button class="sw" class:on={g.skin === k} style={swatch(s.hex)} title={pretty(s.id)} aria-label={pretty(s.id)}
                  onclick={() => change((x) => (x.skin = k))}></button>
              {/each}
            </div>
            <label class="ac-row">
              <span>{$t("city3d.avatar.skinVariation")}</span>
              <input type="range" min="0" max="255" value={g.skinVar} oninput={(e) => change((x) => (x.skinVar = +e.currentTarget.value))} />
            </label>
            <label class="ac-row">
              <span>{$t("city3d.avatar.hair")}</span>
              <select value={g.hair} onchange={(e) => change((x) => (x.hair = +e.currentTarget.value))}>
                {#each catalog.hair_styles as h, k (h.id)}<option value={k}>{pretty(h.id)}</option>{/each}
              </select>
            </label>
            <div class="ac-swatches">
              {#each catalog.hair_colors as h, k (h.id)}
                <button class="sw" class:on={g.hairColor === k} style={swatch(h.hex)} title={pretty(h.id)} aria-label={pretty(h.id)}
                  onclick={() => change((x) => (x.hairColor = k))}></button>
              {/each}
            </div>
            <label class="ac-row">
              <span>{$t("city3d.avatar.brows")}</span>
              <select value={g.brows} onchange={(e) => change((x) => (x.brows = +e.currentTarget.value))}>
                {#each catalog.brows as b, k (b)}<option value={k}>{pretty(b)}</option>{/each}
              </select>
            </label>
            <label class="ac-row">
              <span>{$t("city3d.avatar.beard")}</span>
              <select value={g.beard} onchange={(e) => change((x) => (x.beard = +e.currentTarget.value))}>
                {#each catalog.beards as b, k (b)}<option value={k}>{pretty(b)}</option>{/each}
              </select>
            </label>
            <span class="ac-label">{$t("city3d.avatar.eyes")}</span>
            <div class="ac-swatches">
              {#each catalog.eye_colors as c, k (c.id)}
                <button class="sw" class:on={g.eyes === k} style={swatch(c.hex)} title={pretty(c.id)} aria-label={pretty(c.id)}
                  onclick={() => change((x) => (x.eyes = k))}></button>
              {/each}
            </div>
          {:else if tab === "clothes"}
            {#each [["top", "tops"], ["bottom", "bottoms"], ["shoes", "shoes"], ["accessory", "accessories"]] as const as [slot, list] (slot)}
              <label class="ac-row">
                <span>{$t(`city3d.avatar.${slot}`)}</span>
                <select value={g[slot]} onchange={(e) => change((x) => (x[slot] = +e.currentTarget.value))}>
                  {#each catalog[list] as p, k (p.id)}
                    <option value={k} disabled={slot === "bottom" && k > 0 && !!catalog.tops[g.top]?.full_body}>{pretty(p.id)}</option>
                  {/each}
                </select>
              </label>
            {/each}
            {#each [0, 1, 2] as ci (ci)}
              <span class="ac-label">{$t(`city3d.avatar.color${ci}`)}</span>
              <div class="ac-swatches small">
                {#each catalog.cloth_colors as c, k (k)}
                  <button class="sw" class:on={g.colors[ci] === k} style={swatch(c)} aria-label={`#${c}`}
                    onclick={() => change((x) => (x.colors[ci] = k))}></button>
                {/each}
              </div>
            {/each}
          {:else}
            {#each [["gait", "gaits"], ["idle", "idles"], ["mood", "moods"]] as const as [field, list] (field)}
              <label class="ac-row">
                <span>{$t(`city3d.avatar.${field}`)}</span>
                <select value={g[field]} onchange={(e) => change((x) => (x[field] = +e.currentTarget.value))}>
                  {#each catalog[list] as v, k (v)}<option value={k}>{pretty(v)}</option>{/each}
                </select>
              </label>
            {/each}
          {/if}
        </div>

        {#if problems.length}
          <ul class="ac-problems">{#each problems as p (p)}<li>{p}</li>{/each}</ul>
        {/if}

        <div class="ac-actions">
          <label class="ac-seed">
            <span>{$t("city3d.avatar.seed")}</span>
            <input type="number" min="0" bind:value={seed} />
          </label>
          <button class="btn btn-sm btn-ghost" onclick={randomize} disabled={busy}>{$t("city3d.avatar.random")}</button>
          <button class="btn btn-sm btn-ghost" onclick={reset} disabled={busy || !dirty}>{$t("city3d.avatar.reset")}</button>
          <button class="btn btn-sm" onclick={doSave} disabled={busy || !!problems.length || !dirty}>{$t("city3d.avatar.save")}</button>
        </div>

        <details class="ac-export">
          <summary>{$t("city3d.avatar.exportTitle")}</summary>
          <label class="ac-row"><span>{$t("city3d.avatar.vrmName")}</span><input type="text" bind:value={vrmName} /></label>
          <label class="ac-row"><span>{$t("city3d.avatar.vrmAuthor")}</span><input type="text" bind:value={vrmAuthor} /></label>
          <p class="ac-hint">{$t("city3d.avatar.exportHint")}</p>
          <button class="btn btn-sm" onclick={doExport} disabled={busy}>{$t("city3d.avatar.export")}</button>
        </details>
        <code class="ac-genome" title={$t("city3d.avatar.genome")}>{hex}</code>
      </div>
    {/if}
  </div>
</div>

<style>
  .ac-wrap {
    container-type: inline-size;
    height: 100%;
  }
  .ac {
    display: grid;
    grid-template-columns: minmax(240px, 1fr) minmax(280px, 360px);
    gap: var(--space-3);
    height: 100%;
    min-height: 420px;
  }
  /* In the city's side panel there is room for one column only. */
  @container (max-width: 560px) {
    .ac {
      grid-template-columns: minmax(0, 1fr);
      height: auto;
    }
    .ac-preview {
      height: 300px;
    }
  }
  .ac-preview {
    position: relative;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    overflow: hidden;
    min-height: 280px;
  }
  .ac-preview canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
  .ac-preview-bar {
    position: absolute;
    left: var(--space-2);
    bottom: var(--space-2);
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: var(--text-xs);
  }
  .ac-check {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    color: var(--text-muted);
  }
  .ac-error {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    color: var(--error);
    font-size: var(--text-xs);
  }
  .ac-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }
  .ac-tabs {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .ac-tabs button {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .ac-tabs button.on {
    background: var(--fill-2);
    color: var(--text);
  }
  .ac-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  .ac-row {
    display: grid;
    grid-template-columns: 1fr 1.4fr;
    gap: var(--space-2);
    align-items: center;
    font-size: var(--text-sm);
  }
  .ac-label {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .ac-hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
    margin: 0;
  }
  .ac-swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .sw {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 1px solid var(--hairline);
    cursor: pointer;
    padding: 0;
  }
  .ac-swatches.small .sw {
    width: 16px;
    height: 16px;
  }
  .sw.on {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .ac-problems {
    color: var(--error);
    font-size: var(--text-xs);
    margin: 0;
    padding-left: var(--space-3);
  }
  .ac-actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex-wrap: wrap;
  }
  .ac-seed {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .ac-seed input {
    width: 90px;
  }
  .ac-export {
    border-top: 1px solid var(--separator);
    padding-top: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .ac-genome {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-dim);
    word-break: break-all;
  }
  @media (max-width: 720px) {
    .ac {
      grid-template-columns: 1fr;
    }
  }
</style>
