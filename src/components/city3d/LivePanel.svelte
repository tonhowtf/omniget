<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import {
    CLIP_CATALOG,
    createGesturePlayer,
    encodePose,
    listCameras,
    startLive,
    validateGesture,
    LiveError,
    type CameraInfo,
    type LivePose,
    type LiveSession,
    type LiveStats,
    type GesturePlayer,
  } from "$lib/city3d/live";
  import type { AssetProgress } from "$lib/city3d/live/assets";

  let {
    onPose,
    onSession,
    onGesture,
  }: {
    /** Every pose, from the camera or from a gesture test. Wire it to `applyPose(id, pose)`. */
    onPose?: (pose: LivePose, source: "camera" | "gesture") => void;
    /** The live session when it starts, null when it stops. */
    onSession?: (session: LiveSession | null) => void;
    /** Called with a validated gesture; when absent the panel plays it through `onPose`. */
    onGesture?: (player: GesturePlayer) => void;
  } = $props();

  let cameras = $state<CameraInfo[]>([]);
  let deviceId = $state<string>("");
  let face = $state(true);
  let body = $state(true);
  let hands = $state(false);
  let mirror = $state(true);
  let running = $state(false);
  let starting = $state(false);
  let errorKey = $state<string | null>(null);
  let errorDetail = $state("");
  let progress = $state<AssetProgress | null>(null);
  let stats = $state<LiveStats | null>(null);
  let frameBytes = $state(0);
  let video = $state<HTMLVideoElement | null>(null);

  let gestureText = $state('{"mode":"clip","clip":"wave","params":{"side":"right","intensity":0.8,"loops":2}}');
  let gestureErrors = $state<string[]>([]);
  let gestureWarnings = $state<string[]>([]);
  let gesturePlaying = $state(false);

  let session: LiveSession | null = null;
  let unsub: (() => void) | null = null;
  let statsTimer: ReturnType<typeof setInterval> | null = null;
  let gestureRaf = 0;

  async function refreshCameras() {
    try {
      cameras = await listCameras();
      if (!deviceId && cameras.length) deviceId = cameras[0].id;
    } catch {
      cameras = [];
    }
  }
  refreshCameras();

  const ERROR_KEYS: Record<string, string> = {
    denied: "city3d.live.error_denied",
    no_camera: "city3d.live.error_no_camera",
    unsupported: "city3d.live.error_unsupported",
    assets: "city3d.live.error_assets",
    vision: "city3d.live.error_vision",
    native: "city3d.live.error_native",
  };

  async function start() {
    if (running || starting) return;
    starting = true;
    errorKey = null;
    progress = null;
    try {
      session = await startLive({
        deviceId: deviceId || null,
        face,
        body,
        hands,
        mirror,
        preview: video,
        onProgress: (p) => (progress = p),
      });
      running = true;
      progress = null;
      unsub = session.subscribe((pose) => {
        onPose?.(pose, "camera");
      });
      statsTimer = setInterval(() => {
        if (!session) return;
        stats = session.stats();
        const p = session.latest();
        if (p) frameBytes = encodePose(p, { actor: 0 }).byteLength;
      }, 500);
      onSession?.(session);
      // Labels show up only after the permission is granted.
      refreshCameras();
    } catch (e) {
      const code = e instanceof LiveError ? e.code : "vision";
      errorKey = ERROR_KEYS[code] ?? ERROR_KEYS.vision;
      errorDetail = String((e as Error)?.message ?? e);
      session = null;
    } finally {
      starting = false;
    }
  }

  async function stop() {
    unsub?.();
    unsub = null;
    if (statsTimer) clearInterval(statsTimer);
    statsTimer = null;
    await session?.stop();
    session = null;
    running = false;
    stats = null;
    onSession?.(null);
  }

  async function toggleTask(which: "face" | "body" | "hands", value: boolean) {
    if (!session) return;
    try {
      await session.setTasks({ [which]: value });
    } catch (e) {
      errorKey = ERROR_KEYS.assets;
      errorDetail = String((e as Error)?.message ?? e);
    }
  }

  function setMirror(value: boolean) {
    mirror = value;
    session?.setSolver({ mirror: value });
  }

  function pickClip(id: string) {
    const spec = CLIP_CATALOG.find((c) => c.id === id);
    const params: Record<string, unknown> = { intensity: 0.8 };
    if (spec?.sided) params.side = "right";
    if (spec?.loops) params.loops = 2;
    gestureText = JSON.stringify({ mode: "clip", clip: id, params });
  }

  function playGesture() {
    const v = validateGesture(gestureText);
    if (!v.ok) {
      gestureErrors = v.errors;
      gestureWarnings = [];
      return;
    }
    gestureErrors = [];
    gestureWarnings = v.warnings;
    const player = createGesturePlayer(v.action);
    if (onGesture) {
      onGesture(player);
      return;
    }
    cancelAnimationFrame(gestureRaf);
    const t0 = performance.now();
    const end = Number.isFinite(player.duration) ? player.duration : 4;
    gesturePlaying = true;
    const step = () => {
      const tt = (performance.now() - t0) / 1000;
      onPose?.(player.sample(Math.min(tt, end)), "gesture");
      if (tt < end) gestureRaf = requestAnimationFrame(step);
      else gesturePlaying = false;
    };
    step();
  }

  onDestroy(() => {
    cancelAnimationFrame(gestureRaf);
    void stop();
  });

  const pct = (p: AssetProgress) => (p.total ? Math.round((p.received / p.total) * 100) : 0);
</script>

<section class="live">
  <header>
    <h3>{$t("city3d.live.title")}</h3>
    <p class="privacy">{$t("city3d.live.privacy")}</p>
  </header>

  <div class="row">
    <label class="field">
      <span>{$t("city3d.live.camera")}</span>
      <select bind:value={deviceId} disabled={running || starting}>
        {#if cameras.length === 0}
          <option value="">{$t("city3d.live.camera_default")}</option>
        {/if}
        {#each cameras as cam (cam.id)}
          <option value={cam.id}>{cam.label}</option>
        {/each}
      </select>
    </label>
    {#if running}
      <button class="btn" onclick={stop}>{$t("city3d.live.stop")}</button>
    {:else}
      <button class="btn primary" onclick={start} disabled={starting}>
        {starting ? $t("city3d.live.starting") : $t("city3d.live.start")}
      </button>
    {/if}
  </div>

  <div class="toggles">
    <label><input type="checkbox" bind:checked={face} onchange={() => toggleTask("face", face)} /> {$t("city3d.live.face")}</label>
    <label><input type="checkbox" bind:checked={body} onchange={() => toggleTask("body", body)} /> {$t("city3d.live.body")}</label>
    <label title={$t("city3d.live.hands_hint")}>
      <input type="checkbox" bind:checked={hands} onchange={() => toggleTask("hands", hands)} /> {$t("city3d.live.hands")}
    </label>
    <label><input type="checkbox" checked={mirror} onchange={(e) => setMirror(e.currentTarget.checked)} /> {$t("city3d.live.mirror")}</label>
  </div>

  {#if progress}
    <div class="progress" role="progressbar" aria-valuenow={pct(progress)} aria-valuemin="0" aria-valuemax="100">
      <span>{$t("city3d.live.downloading")} {progress.name}</span>
      <div class="bar"><div style:width="{pct(progress)}%"></div></div>
    </div>
  {/if}

  <!-- Local preview only: this element is never captured or sent. -->
  <video class="preview" class:mirror class:hidden={!running} bind:this={video} muted playsinline></video>

  {#if stats}
    <dl class="stats">
      <div><dt>{$t("city3d.live.mode")}</dt><dd>{stats.mode} · {stats.delegate}</dd></div>
      <div><dt>{$t("city3d.live.fps")}</dt><dd>{stats.fps.toFixed(0)}</dd></div>
      <div><dt>{$t("city3d.live.infer")}</dt><dd>{stats.inferMs.toFixed(1)} ms</dd></div>
      <div><dt>{$t("city3d.live.latency")}</dt><dd>{stats.latencyMs.toFixed(1)} ms</dd></div>
      <div><dt>{$t("city3d.live.frame_size")}</dt><dd>{frameBytes} B</dd></div>
    </dl>
  {/if}

  {#if errorKey}
    <p class="error" title={errorDetail}>{$t(errorKey)}</p>
  {/if}

  <div class="gesture">
    <h4>{$t("city3d.live.gesture_title")}</h4>
    <div class="chips">
      {#each CLIP_CATALOG as clip (clip.id)}
        <button class="chip" title={clip.description} onclick={() => pickClip(clip.id)}>{clip.id}</button>
      {/each}
    </div>
    <textarea bind:value={gestureText} rows="4" spellcheck="false" aria-label={$t("city3d.live.gesture_input")}></textarea>
    <button class="btn" onclick={playGesture} disabled={gesturePlaying}>{$t("city3d.live.gesture_play")}</button>
    {#if gestureErrors.length}
      <ul class="error">
        {#each gestureErrors as err, i (i)}<li>{err}</li>{/each}
      </ul>
    {/if}
    {#if gestureWarnings.length}
      <ul class="warn">
        {#each gestureWarnings as w, i (i)}<li>{w}</li>{/each}
      </ul>
    {/if}
  </div>
</section>

<style>
  .live {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    color: var(--text);
    font-size: var(--text-sm);
  }
  h3,
  h4 {
    margin: 0;
    font-size: var(--text-md);
  }
  h4 {
    font-size: var(--text-sm);
  }
  .privacy {
    margin: var(--space-1) 0 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .row {
    display: flex;
    gap: var(--space-2);
    align-items: flex-end;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
  }
  .field span,
  dt {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  select,
  textarea {
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: var(--radius-md);
    color: var(--text);
    padding: var(--space-1) var(--space-2);
    font: inherit;
  }
  textarea {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    resize: vertical;
  }
  .btn {
    height: var(--control-h);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--button-stroke);
    background: var(--button);
    color: var(--button-text);
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--button-hover);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
    border-color: transparent;
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .btn:focus-visible,
  .chip:focus-visible {
    outline: 2px solid var(--focus-ring);
  }
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
  .toggles label {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .progress span {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .bar {
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
    margin-top: var(--space-1);
  }
  .bar div {
    height: 100%;
    background: var(--accent);
    transition: width var(--duration-fast) linear;
  }
  .preview {
    width: 160px;
    aspect-ratio: 4 / 3;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    object-fit: cover;
  }
  .preview.mirror {
    transform: scaleX(-1);
  }
  .preview.hidden {
    display: none;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(90px, 1fr));
    gap: var(--space-2);
    margin: 0;
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .error {
    color: var(--error);
    font-size: var(--text-xs);
    margin: 0;
  }
  .warn {
    color: var(--warning);
    font-size: var(--text-xs);
    margin: 0;
  }
  ul {
    padding-left: var(--space-4);
  }
  .gesture {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    border-top: 1px solid var(--separator);
    padding-top: var(--space-3);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .chip {
    border: 1px solid var(--border);
    background: var(--fill-1);
    color: var(--text);
    border-radius: var(--radius-full);
    padding: 2px var(--space-2);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--fill-2);
  }
</style>
