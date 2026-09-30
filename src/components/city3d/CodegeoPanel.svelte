<script lang="ts">
  // Script editor for geometry by code (agent E): write a Rhai script against
  // the city kit, run it in the Rust sandbox, preview the GLB in three.js
  // with a style palette, read errors by line and save it as a typology.
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/i18n';
  import {
    CODEGEO_CLASSES,
    KIT_STYLES,
    STYLE_PALETTES,
    base64ToArrayBuffer,
    codegeoExamples,
    deleteTypology,
    errorLabel,
    glbToObject,
    listTypologies,
    runCodegeo,
    saveTypology,
    type CodegeoExample,
    type CodegeoRun,
    type KitStyle,
    type ScriptError,
    type Typology,
  } from '$lib/city3d/codegeo';

  let {
    style = 'vila-colonial',
    onsaved,
  }: { style?: string; onsaved?: (t: Typology) => void } = $props();

  const STARTER = `// A small house: walls, a door on the south side, two windows, a roof.
let w = 4.0; let d = 3.2; let h = 2.8;
let body = box([w, h, d], #{ mat: "wall" });
body = door(body, #{ at: [0.0, 0.0, d / 2.0], face: "s" });
for x in [-1.3, 1.3] {
    body = window(body, #{ at: [x, 0.9, d / 2.0], face: "s", shutters: true });
}
let roof = gable(#{ w: w, d: d, h: 1.4, eave: 0.4, at: [0.0, h, 0.0] });
[body, roof, box([w + 0.1, 0.4, d + 0.1], #{ mat: "stone" })]
`;

  let script = $state(STARTER);
  let cls = $state('building:house');
  let paletteStyle = $state<KitStyle>(
    (KIT_STYLES as readonly string[]).includes(style) ? (style as KitStyle) : 'vila-colonial',
  );
  let running = $state(false);
  let result = $state<CodegeoRun | null>(null);
  let errors = $state<ScriptError[]>([]);
  let failure = $state('');
  let examples = $state<CodegeoExample[]>([]);
  let typologies = $state<Typology[]>([]);
  let saveName = $state('');
  let savedNote = $state('');
  let editor = $state<HTMLTextAreaElement | null>(null);
  let host = $state<HTMLDivElement | null>(null);

  // three.js preview (created lazily on mount; nothing runs in the webview
  // but drawing).
  type Three = typeof import('three');
  let THREE: Three | null = null;
  let renderer: import('three').WebGLRenderer | null = null;
  let scene: import('three').Scene | null = null;
  let camera: import('three').PerspectiveCamera | null = null;
  let controls: { update(): void; dispose(): void; target: import('three').Vector3 } | null = null;
  let current: import('three').Object3D | null = null;
  let lastGlb: ArrayBuffer | null = null;
  let raf = 0;
  let ro: ResizeObserver | null = null;

  onMount(() => {
    void initPreview();
    void codegeoExamples().then((e) => (examples = e)).catch(() => {});
    void refreshTypologies();
  });

  onDestroy(() => {
    cancelAnimationFrame(raf);
    ro?.disconnect();
    controls?.dispose();
    clearCurrent();
    renderer?.dispose();
    renderer?.domElement.remove();
  });

  async function initPreview() {
    if (!host) return;
    THREE = await import('three');
    const { OrbitControls } = await import('three/examples/jsm/controls/OrbitControls.js');
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.setClearColor(new THREE.Color('#ECE8DE'));
    host.appendChild(renderer.domElement);
    scene = new THREE.Scene();
    scene.add(new THREE.HemisphereLight('#FFF7E8', '#8A8070', 1.6));
    const sun = new THREE.DirectionalLight('#FFFFFF', 1.8);
    sun.position.set(6, 10, 4);
    scene.add(sun);
    const grid = new THREE.GridHelper(20, 10, '#B8AE9C', '#D6CFBF');
    scene.add(grid);
    camera = new THREE.PerspectiveCamera(40, 1, 0.1, 500);
    camera.position.set(8, 7, 10);
    const c = new OrbitControls(camera, renderer.domElement);
    c.enableDamping = true;
    controls = c;
    ro = new ResizeObserver(resize);
    ro.observe(host);
    resize();
    const loop = () => {
      raf = requestAnimationFrame(loop);
      controls?.update();
      if (renderer && scene && camera) renderer.render(scene, camera);
    };
    loop();
    if (lastGlb) await show(lastGlb);
  }

  function resize() {
    if (!host || !renderer || !camera) return;
    const w = Math.max(host.clientWidth, 1);
    const h = Math.max(host.clientHeight, 1);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }

  function clearCurrent() {
    if (!current || !scene) return;
    scene.remove(current);
    current.traverse((o) => {
      const m = o as import('three').Mesh;
      if (m.isMesh) {
        m.geometry.dispose();
        const mats = Array.isArray(m.material) ? m.material : [m.material];
        for (const x of mats) x.dispose();
      }
    });
    current = null;
  }

  async function show(buf: ArrayBuffer) {
    lastGlb = buf;
    if (!THREE || !scene || !camera || !controls) return;
    const obj = await glbToObject(buf.slice(0), STYLE_PALETTES[paletteStyle]);
    clearCurrent();
    current = obj;
    scene.add(obj);
    const box = new THREE.Box3().setFromObject(obj);
    const size = box.getSize(new THREE.Vector3());
    const center = box.getCenter(new THREE.Vector3());
    const r = Math.max(size.x, size.y, size.z, 1);
    controls.target.copy(center);
    camera.position.set(center.x + r * 1.3, center.y + r * 1.0, center.z + r * 1.6);
    camera.near = r / 100;
    camera.far = r * 50;
    camera.updateProjectionMatrix();
  }

  async function run() {
    if (running) return;
    running = true;
    failure = '';
    savedNote = '';
    try {
      const r = await runCodegeo(script, cls);
      result = r;
      errors = r.errors;
      if (!r.errors.length && r.glb_base64) await show(base64ToArrayBuffer(r.glb_base64));
    } catch (e) {
      failure = String(e);
    } finally {
      running = false;
    }
  }

  async function refreshTypologies() {
    try {
      typologies = await listTypologies();
    } catch {
      typologies = [];
    }
  }

  async function save() {
    const name = saveName.trim();
    if (!name) return;
    running = true;
    savedNote = '';
    try {
      const r = await saveTypology(name, script, cls);
      if (r.typology) {
        savedNote = r.typology.typology;
        saveName = '';
        onsaved?.(r.typology);
        await refreshTypologies();
      } else {
        errors = r.errors;
      }
    } catch (e) {
      failure = String(e);
    } finally {
      running = false;
    }
  }

  async function remove(tp: Typology) {
    await deleteTypology(tp.typology).catch(() => false);
    await refreshTypologies();
  }

  function load(src: string, klass: string) {
    script = src;
    cls = klass;
    void run();
  }

  function jumpTo(e: ScriptError) {
    if (!editor || e.line == null) return;
    const lines = script.split('\n');
    let pos = 0;
    for (let i = 0; i < Math.min(e.line - 1, lines.length); i++) pos += lines[i].length + 1;
    pos += Math.max((e.column ?? 1) - 1, 0);
    editor.focus();
    editor.setSelectionRange(pos, pos);
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void run();
    }
    // Tab indents instead of leaving the editor.
    if (e.key === 'Tab' && editor) {
      e.preventDefault();
      const { selectionStart: s, selectionEnd: end } = editor;
      script = script.slice(0, s) + '    ' + script.slice(end);
      requestAnimationFrame(() => editor?.setSelectionRange(s + 4, s + 4));
    }
  }

  $effect(() => {
    // Recolour the preview when the palette changes.
    void paletteStyle;
    if (lastGlb) void show(lastGlb);
  });
</script>

<section class="codegeo" aria-label={$t('city3d.codegeo.title')}>
  <header class="bar">
    <h2>{$t('city3d.codegeo.title')}</h2>
    <div class="examples" role="group" aria-label={$t('city3d.codegeo.examples')}>
      {#each examples as ex (ex.id)}
        <button type="button" class="chip" onclick={() => load(ex.script, ex.class)}>
          {$t(`city3d.codegeo.example_${ex.id}`)}
        </button>
      {/each}
    </div>
  </header>

  <div class="grid">
    <div class="left">
      <textarea
        bind:this={editor}
        bind:value={script}
        class="editor"
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
        aria-label={$t('city3d.codegeo.script')}
        onkeydown={onKey}
      ></textarea>
      <div class="controls">
        <label>
          <span>{$t('city3d.codegeo.class')}</span>
          <select bind:value={cls}>
            {#each CODEGEO_CLASSES as c (c)}
              <option value={c}>{c}</option>
            {/each}
          </select>
        </label>
        <button type="button" class="primary" onclick={run} disabled={running}>
          {running ? $t('city3d.codegeo.running') : $t('city3d.codegeo.run')}
        </button>
      </div>

      <div class="messages" aria-live="polite">
        {#if failure}
          <p class="error">{failure}</p>
        {/if}
        {#if errors.length}
          <h3>{$t('city3d.codegeo.errors')}</h3>
          <ul class="errors">
            {#each errors as e, i (i)}
              <li>
                <button type="button" class="err" onclick={() => jumpTo(e)} disabled={e.line == null}>
                  <span class="where">{errorLabel(e)}</span>
                  <span>{e.message}</span>
                </button>
              </li>
            {/each}
          </ul>
        {:else if result && !failure}
          <p class="ok">
            {$t('city3d.codegeo.stats', {
              triangles: result.triangles,
              anchors: result.anchors.length,
              width: result.footprint?.w ?? 0,
              depth: result.footprint?.d ?? 0,
              ms: Math.round(result.ms),
            })}
          </p>
          {#each result.warnings as w, i (i)}
            <p class="warn">{w}</p>
          {/each}
        {/if}
      </div>
    </div>

    <div class="right">
      <div class="preview" bind:this={host}>
        {#if !result || errors.length}
          <p class="empty">{$t('city3d.codegeo.preview_empty')}</p>
        {/if}
      </div>
      <label class="palette">
        <span>{$t('city3d.codegeo.palette')}</span>
        <select bind:value={paletteStyle}>
          {#each KIT_STYLES as s (s)}
            <option value={s}>{s}</option>
          {/each}
        </select>
      </label>
      {#if result && !errors.length && result.anchors.length}
        <ul class="anchors">
          {#each result.anchors as a, i (i)}
            <li><code>{a.action}</code> · {$t('city3d.codegeo.anchor_at', { col: a.cell[0], row: a.cell[1], level: a.level })}</li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>

  <footer class="save">
    <div class="row">
      <input
        type="text"
        bind:value={saveName}
        placeholder={$t('city3d.codegeo.name_placeholder')}
        aria-label={$t('city3d.codegeo.name_placeholder')}
        maxlength="80"
      />
      <button type="button" onclick={save} disabled={running || !saveName.trim()}>
        {$t('city3d.codegeo.save')}
      </button>
    </div>
    {#if savedNote}
      <p class="ok">{$t('city3d.codegeo.saved')} <code>{savedNote}</code></p>
    {/if}
    <h3>{$t('city3d.codegeo.typologies')}</h3>
    {#if typologies.length === 0}
      <p class="dim">{$t('city3d.codegeo.empty_typologies')}</p>
    {:else}
      <ul class="typologies">
        {#each typologies as tp (tp.hash)}
          <li>
            <button type="button" class="link" onclick={() => load(tp.script, tp.class)}>{tp.name}</button>
            <span class="dim">{tp.class} · {tp.triangles} △ · {tp.footprint.w}×{tp.footprint.d}</span>
            <button type="button" class="ghost" onclick={() => remove(tp)} aria-label={$t('city3d.codegeo.delete')}>
              {$t('city3d.codegeo.delete')}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </footer>
</section>

<style>
  .codegeo {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    color: var(--text, inherit);
    font-size: var(--text-sm, 13px);
    min-width: 0;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg, 16px);
  }
  h3 {
    margin: var(--space-2, 8px) 0 var(--space-1, 4px);
    font-size: var(--text-xs, 12px);
    color: var(--text-dim, #777);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .examples {
    display: flex;
    gap: var(--space-1, 4px);
    flex-wrap: wrap;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-3, 12px);
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .left,
  .right {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    min-width: 0;
  }
  .editor {
    min-height: 320px;
    resize: vertical;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12.5px;
    line-height: 1.5;
    tab-size: 4;
    padding: var(--space-2, 8px);
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--separator, rgba(0, 0, 0, 0.12));
    background: var(--fill-1, rgba(0, 0, 0, 0.03));
    color: inherit;
    white-space: pre;
    overflow: auto;
  }
  .controls,
  .row {
    display: flex;
    gap: var(--space-2, 8px);
    align-items: center;
    flex-wrap: wrap;
  }
  label {
    display: flex;
    gap: var(--space-1, 4px);
    align-items: center;
  }
  label span {
    color: var(--text-dim, #777);
  }
  select,
  input,
  button {
    font: inherit;
    color: inherit;
    border-radius: var(--radius-sm, 6px);
    border: 1px solid var(--separator, rgba(0, 0, 0, 0.12));
    background: var(--fill-1, transparent);
    padding: 4px 10px;
  }
  input {
    flex: 1;
    min-width: 160px;
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button:focus-visible,
  select:focus-visible,
  input:focus-visible,
  .editor:focus-visible {
    outline: 2px solid var(--accent, #4a7);
    outline-offset: 1px;
  }
  .primary {
    background: var(--accent, #3b7);
    color: var(--accent-text, #fff);
    border-color: transparent;
  }
  .chip {
    border-radius: var(--radius-full, 999px);
    padding: 2px 12px;
  }
  .preview {
    position: relative;
    aspect-ratio: 4 / 3;
    border-radius: var(--radius-md, 8px);
    overflow: hidden;
    border: 1px solid var(--separator, rgba(0, 0, 0, 0.12));
    background: #ece8de;
  }
  .preview :global(canvas) {
    display: block;
    width: 100%;
    height: 100%;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: #6f675a;
    pointer-events: none;
    text-align: center;
    padding: var(--space-3, 12px);
  }
  .messages p {
    margin: var(--space-1, 4px) 0;
  }
  .errors,
  .anchors,
  .typologies {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .err {
    display: flex;
    gap: var(--space-2, 8px);
    width: 100%;
    text-align: left;
    border-color: transparent;
    background: var(--fill-2, rgba(200, 60, 60, 0.08));
  }
  .where {
    font-family: var(--font-mono, monospace);
    color: var(--error, #c33);
    flex: none;
  }
  .error {
    color: var(--error, #c33);
  }
  .warn {
    color: var(--warning, #b80);
  }
  .ok {
    color: var(--success, #3a6);
  }
  .dim {
    color: var(--text-dim, #777);
  }
  .typologies li {
    display: flex;
    gap: var(--space-2, 8px);
    align-items: center;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    text-decoration: underline;
  }
  .ghost {
    margin-left: auto;
    border-color: transparent;
    background: none;
    color: var(--text-dim, #777);
  }
  code {
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
    word-break: break-all;
  }
</style>
