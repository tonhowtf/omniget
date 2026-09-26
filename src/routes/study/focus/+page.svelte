<script lang="ts">
  import { goto } from "$app/navigation";
  import { onDestroy, onMount } from "svelte";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { awardXp, bumpCounter } from "$lib/study-gamification";
  import { emitFocusBreakStart } from "$lib/study-focus-bridge";
  import { musicPlayer } from "$lib/study-music/player-store.svelte";
  import PlayerBar from "$lib/study-music-components/PlayerBar.svelte";
  import { t } from "$lib/i18n";

  type PresetKind = "pomodoro" | "short_break" | "long_break" | "deep" | "stopwatch";
  type Panel = "settings" | "history" | "intent" | null;
  type SettingsTab = "timer" | "appearance" | "sounds" | "automation";
  type DisplayMode = "standard" | "line";
  type SceneId = "room" | "garden" | "night" | "library" | "cafe" | "ocean";
  type FocusMixId = "lofi" | "synthwave" | "rainy" | "deep";
  type AmbientId = "rain" | "cafe" | "keyboard" | "waves" | "brown";

  type Current = {
    id: number;
    preset: string;
    started_at: string;
    elapsed_minutes: number;
    subject_id?: number | null;
  } | null;

  type Session = {
    id: number;
    preset: string;
    started_at: string;
    ended_at: string | null;
    minutes: number | null;
    interrupted?: boolean;
    subject_name?: string | null;
  };

  type History = {
    sessions: Session[];
    today_minutes: number;
    week_minutes: number;
  };

  type XpState = {
    xp: number;
    level: number;
    xp_to_next: number;
    level_progress_pct: number;
  };

  type FocusSettings = {
    pomodoro_minutes: number;
    short_break_minutes: number;
    long_break_minutes: number;
    cycles_before_long_break: number;
    notify_end: boolean;
    notify_sound: boolean;
    sound_volume: number;
    auto_pause_player: boolean;
    auto_start_breaks: boolean;
    auto_start_focus: boolean;
  };

  const RPC_TEXT_KEY = "study.focus.rpc_text.v1";
  const SCENE_KEY = "study.focus.scene.v1";
  const DISPLAY_KEY = "study.focus.display.v1";
  const AUDIO_PREFS_KEY = "study.focus.audio.v1";
  const AUTO_HIDE_KEY = "study.focus.auto_hide.v1";

  const DEFAULT_FOCUS_SETTINGS: FocusSettings = {
    pomodoro_minutes: 25,
    short_break_minutes: 5,
    long_break_minutes: 15,
    cycles_before_long_break: 4,
    notify_end: false,
    notify_sound: true,
    sound_volume: 60,
    auto_pause_player: true,
    auto_start_breaks: false,
    auto_start_focus: false,
  };

  const MODES: {
    kind: PresetKind;
    labelKey: string;
    fallbackMinutes: number;
    badge: string;
  }[] = [
    { kind: "pomodoro", labelKey: "study.focus.preset_pomodoro", fallbackMinutes: 25, badge: "01" },
    { kind: "short_break", labelKey: "study.focus.preset_short_break", fallbackMinutes: 5, badge: "02" },
    { kind: "long_break", labelKey: "study.focus.preset_long_break", fallbackMinutes: 15, badge: "03" },
    { kind: "deep", labelKey: "study.focus.preset_deep", fallbackMinutes: 50, badge: "04" },
    { kind: "stopwatch", labelKey: "study.focus.preset_custom", fallbackMinutes: 0, badge: "05" },
  ];

  const SCENES: { id: SceneId; label: string; hint: string }[] = [
    { id: "room", label: "Room", hint: "warm desk" },
    { id: "library", label: "Library", hint: "quiet stacks" },
    { id: "cafe", label: "Cafe", hint: "late table" },
    { id: "garden", label: "Garden", hint: "green shade" },
    { id: "night", label: "Night", hint: "blue hour" },
    { id: "ocean", label: "Ocean", hint: "low tide" },
  ];

  const FOCUS_MIXES: {
    id: FocusMixId;
    label: string;
    subtitle: string;
    tones: number[];
    mood: string;
  }[] = [
    { id: "lofi", label: "Lofi Desk", subtitle: "soft pad, slow pulse", tones: [174.61, 261.63, 329.63], mood: "study" },
    { id: "synthwave", label: "Synth Grid", subtitle: "bright analog wash", tones: [130.81, 196, 392], mood: "flow" },
    { id: "rainy", label: "Rain Window", subtitle: "darker keys, gentle noise", tones: [146.83, 220, 349.23], mood: "calm" },
    { id: "deep", label: "Deep Work", subtitle: "minimal drone", tones: [110, 164.81, 220], mood: "quiet" },
  ];

  const AMBIENT_SOUNDS: { id: AmbientId; label: string; filter: BiquadFilterType; frequency: number }[] = [
    { id: "rain", label: "Rain", filter: "bandpass", frequency: 1800 },
    { id: "cafe", label: "Cafe", filter: "lowpass", frequency: 950 },
    { id: "keyboard", label: "Keyboard", filter: "highpass", frequency: 2200 },
    { id: "waves", label: "Waves", filter: "lowpass", frequency: 480 },
    { id: "brown", label: "Brown noise", filter: "lowpass", frequency: 260 },
  ];

  const SETTINGS_TABS: {
    id: SettingsTab;
    labelKey: string;
    icon: string;
  }[] = [
    { id: "timer", labelKey: "study.focus.acc_cycles_title", icon: "M12 6v6l4 2 M12 22a10 10 0 1 1 0-20 10 10 0 0 1 0 20z" },
    { id: "appearance", labelKey: "settings.cat_appearance", icon: "M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.2a1.8 1.8 0 0 0-1.4 2.9l.3.4A1.7 1.7 0 0 1 12 22z M7 12h.01 M9 7h.01 M14 6h.01 M18 10h.01" },
    { id: "sounds", labelKey: "study.focus.acc_sounds_title", icon: "M11 5 6.5 8.5H3v7h3.5L11 19V5z M16 9a5 5 0 0 1 0 6 M19 6a9 9 0 0 1 0 12" },
    { id: "automation", labelKey: "study.focus.config_autostart", icon: "M5 12a7 7 0 0 1 12-5 M19 5v5h-5 M19 12a7 7 0 0 1-12 5 M5 19v-5h5" },
  ];

  let current = $state<Current>(null);
  let history = $state<History>({ sessions: [], today_minutes: 0, week_minutes: 0 });
  let xpState = $state<XpState | null>(null);
  let loading = $state(true);
  let tick = $state(0);
  let poll: number | null = null;
  let refreshPoll: number | null = null;
  let panel = $state<Panel>(null);
  let settingsTab = $state<SettingsTab>("timer");
  let selectedKind = $state<PresetKind>("pomodoro");
  let displayMode = $state<DisplayMode>("standard");
  let sceneId = $state<SceneId>("room");
  let rpcCustomText = $state("");
  let activeMix = $state<FocusMixId>("lofi");
  let focusRadioPlaying = $state(false);
  let focusRadioVolume = $state(36);
  let ambientPower = $state(false);
  let ambientVolumes = $state<Record<AmbientId, number>>({
    rain: 18,
    cafe: 0,
    keyboard: 0,
    waves: 0,
    brown: 12,
  });
  let autoHideUi = $state(true);
  let uiIdle = $state(false);
  let saveTimer: number | null = null;
  let savingState = $state<"idle" | "saving" | "saved">("idle");
  let idleTimer: number | null = null;
  let audioCtx: AudioContext | null = null;
  let radioGain: GainNode | null = null;
  let radioNodes: (OscillatorNode | AudioBufferSourceNode)[] = [];
  const ambientNodes = new Map<AmbientId, { source: AudioBufferSourceNode; gain: GainNode; filter: BiquadFilterNode }>();

  let settings = $state<FocusSettings>({ ...DEFAULT_FOCUS_SETTINGS });

  const activeKind = $derived(current ? presetKindFromId(current.preset) : selectedKind);
  const activeMode = $derived(MODES.find((m) => m.kind === activeKind) ?? MODES[0]);
  const targetMinutes = $derived(current ? presetMinutesFromId(current.preset) : minutesForKind(selectedKind));

  const elapsedSec = $derived.by(() => {
    void tick;
    if (!current) return 0;
    const startedMs = Date.parse(current.started_at.replace(" ", "T") + "Z");
    return Math.max(0, Math.floor((Date.now() - startedMs) / 1000));
  });

  const remainingSec = $derived.by(() => {
    void tick;
    if (!current || targetMinutes <= 0) return null;
    return Math.max(0, targetMinutes * 60 - elapsedSec);
  });

  const progressPct = $derived.by(() => {
    void tick;
    if (!current || targetMinutes <= 0) return 0;
    return Math.min(100, Math.max(0, (elapsedSec / (targetMinutes * 60)) * 100));
  });

  const isLowTime = $derived(remainingSec != null && remainingSec > 0 && remainingSec <= 60);
  const isFinished = $derived(remainingSec === 0);
  const timerText = $derived(remainingSec != null ? fmt(remainingSec) : fmt(elapsedSec));
  const goalMinutes = $derived(Math.max(1, settings.pomodoro_minutes * settings.cycles_before_long_break));
  const goalPct = $derived(Math.min(100, Math.round((history.today_minutes / goalMinutes) * 100)));
  const cyclesToday = $derived(
    Math.floor(history.today_minutes / Math.max(1, settings.pomodoro_minutes)),
  );
  const activeMixInfo = $derived(FOCUS_MIXES.find((mix) => mix.id === activeMix) ?? FOCUS_MIXES[0]);
  const uiHidden = $derived(Boolean(autoHideUi && current && uiIdle && !panel));

  let prevFinished = false;
  $effect(() => {
    const finished = isFinished;
    if (finished && !prevFinished && settings.auto_pause_player && current) {
      emitFocusBreakStart({ sessionId: current.id, reason: "session_ended" });
    }
    prevFinished = finished;
  });

  function clamp(n: number, min: number, max: number) {
    return Math.max(min, Math.min(max, Number.isFinite(n) ? n : min));
  }

  function adjustNumberSetting(
    key:
      | "pomodoro_minutes"
      | "short_break_minutes"
      | "long_break_minutes"
      | "cycles_before_long_break",
    delta: number,
    min: number,
    max: number,
  ) {
    settings[key] = clamp(Number(settings[key]) + delta, min, max);
    void saveSettings();
  }

  function resetFocusSettings() {
    settings = { ...DEFAULT_FOCUS_SETTINGS };
    persistScene("room");
    persistDisplay("standard");
    activeMix = "lofi";
    focusRadioVolume = 36;
    ambientVolumes = { rain: 18, cafe: 0, keyboard: 0, waves: 0, brown: 12 };
    persistAutoHide(true);
    stopFocusRadio(false);
    stopAmbient(false);
    saveAudioPrefs();
    void saveSettings();
  }

  function minutesForKind(kind: PresetKind): number {
    if (kind === "pomodoro") return clamp(settings.pomodoro_minutes, 1, 120);
    if (kind === "short_break") return clamp(settings.short_break_minutes, 1, 60);
    if (kind === "long_break") return clamp(settings.long_break_minutes, 1, 60);
    if (kind === "deep") return 50;
    return 0;
  }

  function presetIdForKind(kind: PresetKind): string {
    if (kind === "pomodoro") return `pomodoro-${minutesForKind(kind)}`;
    if (kind === "short_break") return `short-break-${minutesForKind(kind)}`;
    if (kind === "long_break") return `long-break-${minutesForKind(kind)}`;
    if (kind === "deep") return "deep-50";
    return "stopwatch";
  }

  function presetKindFromId(id: string): PresetKind {
    if (id.startsWith("short-break")) return "short_break";
    if (id.startsWith("long-break")) return "long_break";
    if (id.startsWith("deep")) return "deep";
    if (id === "stopwatch") return "stopwatch";
    return "pomodoro";
  }

  function presetMinutesFromId(id: string): number {
    if (id === "stopwatch") return 0;
    const match = id.match(/-(\d+)$/);
    if (match) return clamp(Number(match[1]), 0, 180);
    const kind = presetKindFromId(id);
    const fallback = MODES.find((m) => m.kind === kind)?.fallbackMinutes ?? 0;
    return fallback;
  }

  function presetLabel(id: string): string {
    const kind = presetKindFromId(id);
    const mode = MODES.find((m) => m.kind === kind);
    const minutes = presetMinutesFromId(id);
    const label = mode ? $t(mode.labelKey) : id;
    if (minutes <= 0) return label;
    return `${label} ${minutes}`;
  }

  function fmt(sec: number): string {
    const s = Math.floor(sec);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const r = s % 60;
    const pad = (n: number) => String(n).padStart(2, "0");
    if (h > 0) return `${pad(h)}:${pad(m)}:${pad(r)}`;
    return `${pad(m)}:${pad(r)}`;
  }

  function formatDate(iso: string): string {
    try {
      return new Date(iso.replace(" ", "T") + "Z").toLocaleString();
    } catch {
      return iso;
    }
  }

  function shouldIgnoreShortcut(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
  }

  function persistScene(next: SceneId) {
    sceneId = next;
    try {
      localStorage.setItem(SCENE_KEY, next);
    } catch {}
  }

  function persistDisplay(next: DisplayMode) {
    displayMode = next;
    try {
      localStorage.setItem(DISPLAY_KEY, next);
    } catch {}
  }

  function persistAutoHide(next: boolean) {
    autoHideUi = next;
    if (!next) uiIdle = false;
    try {
      localStorage.setItem(AUTO_HIDE_KEY, next ? "1" : "0");
    } catch {}
  }

  function saveAudioPrefs() {
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(
        AUDIO_PREFS_KEY,
        JSON.stringify({
          activeMix,
          focusRadioVolume,
          ambientPower,
          ambientVolumes,
        }),
      );
    } catch {}
  }

  function isSceneId(value: string | null): value is SceneId {
    return !!value && SCENES.some((scene) => scene.id === value);
  }

  function isFocusMixId(value: unknown): value is FocusMixId {
    return typeof value === "string" && FOCUS_MIXES.some((mix) => mix.id === value);
  }

  function loadLocalPrefs() {
    if (typeof localStorage === "undefined") return;
    try {
      rpcCustomText = localStorage.getItem(RPC_TEXT_KEY) ?? "";
      const storedScene = localStorage.getItem(SCENE_KEY);
      if (isSceneId(storedScene)) {
        sceneId = storedScene;
      }
      const storedDisplay = localStorage.getItem(DISPLAY_KEY);
      if (storedDisplay === "standard" || storedDisplay === "line") {
        displayMode = storedDisplay;
      }
      autoHideUi = localStorage.getItem(AUTO_HIDE_KEY) !== "0";
      const audioPrefs = JSON.parse(localStorage.getItem(AUDIO_PREFS_KEY) ?? "{}") as {
        activeMix?: unknown;
        focusRadioVolume?: unknown;
        ambientPower?: unknown;
        ambientVolumes?: Partial<Record<AmbientId, unknown>>;
      };
      if (isFocusMixId(audioPrefs.activeMix)) activeMix = audioPrefs.activeMix;
      if (typeof audioPrefs.focusRadioVolume === "number") {
        focusRadioVolume = clamp(audioPrefs.focusRadioVolume, 0, 100);
      }
      // Browser audio needs an explicit user gesture after reload, so we keep the mix saved but stopped.
      ambientPower = false;
      if (audioPrefs.ambientVolumes) {
        for (const sound of AMBIENT_SOUNDS) {
          const value = audioPrefs.ambientVolumes[sound.id];
          if (typeof value === "number") ambientVolumes[sound.id] = clamp(value, 0, 100);
        }
      }
    } catch {
      rpcCustomText = "";
    }
  }

  function saveRpcText(value: string) {
    rpcCustomText = value;
    if (typeof localStorage === "undefined") return;
    try {
      if (value.trim()) localStorage.setItem(RPC_TEXT_KEY, value.trim());
      else localStorage.removeItem(RPC_TEXT_KEY);
    } catch {}
  }

  function ensureAudioContext(): AudioContext | null {
    if (typeof window === "undefined") return null;
    if (audioCtx) return audioCtx;
    const AudioCtor =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioCtor) return null;
    audioCtx = new AudioCtor();
    return audioCtx;
  }

  function createNoiseBuffer(ctx: AudioContext) {
    const length = ctx.sampleRate * 2;
    const buffer = ctx.createBuffer(1, length, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < length; i += 1) {
      data[i] = Math.random() * 2 - 1;
    }
    return buffer;
  }

  function stopFocusRadio(persist = true) {
    for (const node of radioNodes) {
      try {
        node.stop();
      } catch {}
      try {
        node.disconnect();
      } catch {}
    }
    radioNodes = [];
    try {
      radioGain?.disconnect();
    } catch {}
    radioGain = null;
    focusRadioPlaying = false;
    if (persist) saveAudioPrefs();
  }

  async function startFocusRadio() {
    const ctx = ensureAudioContext();
    if (!ctx) return;
    if (ctx.state === "suspended") await ctx.resume();
    stopFocusRadio(false);

    const mix = activeMixInfo;
    radioGain = ctx.createGain();
    radioGain.gain.value = (focusRadioVolume / 100) * 0.22;
    radioGain.connect(ctx.destination);

    mix.tones.forEach((freq, idx) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = idx === 0 ? "triangle" : "sine";
      osc.frequency.value = freq;
      osc.detune.value = idx * 4 - 5;
      gain.gain.value = idx === 0 ? 0.24 : 0.11;
      osc.connect(gain);
      gain.connect(radioGain!);
      osc.start();
      radioNodes.push(osc);
    });

    const texture = ctx.createBufferSource();
    const filter = ctx.createBiquadFilter();
    const textureGain = ctx.createGain();
    texture.buffer = createNoiseBuffer(ctx);
    texture.loop = true;
    filter.type = activeMix === "synthwave" ? "highpass" : "lowpass";
    filter.frequency.value = activeMix === "deep" ? 220 : activeMix === "synthwave" ? 1800 : 620;
    textureGain.gain.value = activeMix === "rainy" ? 0.1 : 0.035;
    texture.connect(filter);
    filter.connect(textureGain);
    textureGain.connect(radioGain);
    texture.start();
    radioNodes.push(texture);

    focusRadioPlaying = true;
    saveAudioPrefs();
  }

  function toggleFocusRadio() {
    if (focusRadioPlaying) stopFocusRadio();
    else void startFocusRadio();
  }

  function selectFocusMix(id: FocusMixId) {
    activeMix = id;
    if (focusRadioPlaying) void startFocusRadio();
    else saveAudioPrefs();
  }

  function setFocusRadioVolume(value: number) {
    focusRadioVolume = clamp(value, 0, 100);
    if (radioGain) radioGain.gain.value = (focusRadioVolume / 100) * 0.22;
    saveAudioPrefs();
  }

  function createAmbientLayer(ctx: AudioContext, sound: (typeof AMBIENT_SOUNDS)[number]) {
    const source = ctx.createBufferSource();
    const filter = ctx.createBiquadFilter();
    const gain = ctx.createGain();
    source.buffer = createNoiseBuffer(ctx);
    source.loop = true;
    filter.type = sound.filter;
    filter.frequency.value = sound.frequency;
    gain.gain.value = 0;
    source.connect(filter);
    filter.connect(gain);
    gain.connect(ctx.destination);
    source.start();
    ambientNodes.set(sound.id, { source, gain, filter });
  }

  async function startAmbient() {
    const ctx = ensureAudioContext();
    if (!ctx) return;
    if (ctx.state === "suspended") await ctx.resume();
    ambientPower = true;
    for (const sound of AMBIENT_SOUNDS) {
      if (!ambientNodes.has(sound.id)) createAmbientLayer(ctx, sound);
    }
    syncAmbientVolumes();
    saveAudioPrefs();
  }

  function stopAmbient(persist = true) {
    for (const layer of ambientNodes.values()) {
      try {
        layer.source.stop();
      } catch {}
      try {
        layer.source.disconnect();
        layer.filter.disconnect();
        layer.gain.disconnect();
      } catch {}
    }
    ambientNodes.clear();
    ambientPower = false;
    if (persist) saveAudioPrefs();
  }

  function syncAmbientVolumes() {
    for (const sound of AMBIENT_SOUNDS) {
      const layer = ambientNodes.get(sound.id);
      if (!layer) continue;
      layer.gain.gain.value = ambientPower ? (ambientVolumes[sound.id] / 100) * 0.28 : 0;
    }
  }

  function toggleAmbientPower() {
    if (ambientPower) stopAmbient();
    else void startAmbient();
  }

  function setAmbientVolume(id: AmbientId, value: number) {
    ambientVolumes[id] = clamp(value, 0, 100);
    syncAmbientVolumes();
    saveAudioPrefs();
  }

  function markUiAwake() {
    if (!autoHideUi) {
      uiIdle = false;
      return;
    }
    uiIdle = false;
    if (idleTimer != null) clearTimeout(idleTimer);
    if (current && !panel) {
      idleTimer = window.setTimeout(() => {
        uiIdle = true;
      }, 4200);
    }
  }

  async function loadSettings() {
    try {
      const all = await pluginInvoke<{ focus?: Partial<FocusSettings> }>(
        "study",
        "study:settings:get",
      );
      if (all?.focus) settings = { ...settings, ...all.focus };
    } catch (e) {
      console.error("loadSettings failed", e);
    }
  }

  async function saveSettings() {
    if (saveTimer != null) clearTimeout(saveTimer);
    savingState = "saving";
    saveTimer = window.setTimeout(async () => {
      try {
        settings = {
          ...settings,
          pomodoro_minutes: clamp(settings.pomodoro_minutes, 1, 120),
          short_break_minutes: clamp(settings.short_break_minutes, 1, 60),
          long_break_minutes: clamp(settings.long_break_minutes, 1, 60),
          cycles_before_long_break: clamp(settings.cycles_before_long_break, 1, 10),
          sound_volume: clamp(settings.sound_volume, 0, 100),
        };
        const all = await pluginInvoke<Record<string, unknown>>("study", "study:settings:get");
        await pluginInvoke("study", "study:settings:save", {
          settings: { ...all, focus: settings },
        });
        savingState = "saved";
        setTimeout(() => (savingState = "idle"), 1500);
      } catch (e) {
        console.error("saveSettings failed", e);
        savingState = "idle";
      }
    }, 350);
  }

  async function refresh() {
    try {
      const [cur, hist] = await Promise.all([
        pluginInvoke<Current>("study", "study:focus:current"),
        pluginInvoke<History>("study", "study:focus:history"),
      ]);
      current = cur;
      history = hist;
    } catch (e) {
      console.error("focus refresh failed", e);
    } finally {
      loading = false;
    }
  }

  async function refreshXpState() {
    try {
      xpState = await pluginInvoke<XpState>("study", "study:gamification:state");
    } catch (e) {
      console.error("xp state fetch failed", e);
    }
  }

  async function start(kind = selectedKind) {
    selectedKind = kind;
    try {
      await pluginInvoke("study", "study:focus:start", { preset: presetIdForKind(kind) });
      panel = null;
      await refresh();
      markUiAwake();
    } catch (e) {
      console.error("focus start failed", e);
    }
  }

  async function stop(forceCompleted: boolean | null = null) {
    if (!current) return;
    const elapsedMin = Math.max(0, Math.floor(elapsedSec / 60));
    const completed = forceCompleted ?? (targetMinutes <= 0 || isFinished);
    try {
      await pluginInvoke("study", "study:focus:stop", { completed });
      await refresh();
      void refreshXpState();
      uiIdle = false;
      if (elapsedMin >= 1) {
        void awardXp("focus_session", elapsedMin, { minutes: elapsedMin, completed });
        void bumpCounter("focus_minutes", elapsedMin);
      }
    } catch (e) {
      console.error("focus stop failed", e);
    }
  }

  function togglePanel(next: Panel) {
    panel = panel === next ? null : next;
    uiIdle = false;
  }

  function exitFullscreen() {
    void goto("/study");
  }

  function onKey(e: KeyboardEvent) {
    if (shouldIgnoreShortcut(e.target)) return;
    markUiAwake();
    if (e.key === "Escape") {
      e.preventDefault();
      if (panel) panel = null;
      else exitFullscreen();
      return;
    }
    if (e.key === " ") {
      e.preventDefault();
      if (current) void stop();
      else void start();
      return;
    }
    if (e.key >= "1" && e.key <= "5") {
      const mode = MODES[Number(e.key) - 1];
      if (mode) selectedKind = mode.kind;
    }
  }

  onMount(() => {
    loadLocalPrefs();
    void loadSettings();
    void refresh();
    void refreshXpState();
    poll = window.setInterval(() => {
      tick += 1;
    }, 1000);
    refreshPoll = window.setInterval(() => {
      void refresh();
      void refreshXpState();
    }, 30_000);
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousemove", markUiAwake);
    window.addEventListener("pointerdown", markUiAwake);
  });

  onDestroy(() => {
    if (poll != null) clearInterval(poll);
    if (refreshPoll != null) clearInterval(refreshPoll);
    if (saveTimer != null) clearTimeout(saveTimer);
    if (idleTimer != null) clearTimeout(idleTimer);
    stopFocusRadio(false);
    stopAmbient(false);
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("mousemove", markUiAwake);
    window.removeEventListener("pointerdown", markUiAwake);
  });
</script>

<svelte:head>
  <title>{$t("study.focus.title")} | OmniGet</title>
</svelte:head>

<div
  class="focus-shell"
  data-scene={sceneId}
  data-active={current ? "1" : "0"}
  data-low-time={isLowTime ? "1" : "0"}
  data-display={displayMode}
  data-ui-idle={uiHidden ? "1" : "0"}
>
  <div class="ambient-scene" aria-hidden="true">
    <div class="scene-window"></div>
    <div class="scene-shelf"></div>
    <div class="scene-desk"></div>
    <div class="scene-lamp"></div>
  </div>
  <div class="scene-shade" aria-hidden="true"></div>

  <header class="focus-status" aria-label={$t("study.focus.title") as string}>
    <a class="focus-brand" href="/study" aria-label={$t("study.hub.title") as string}>
      <span class="brand-mark">OG</span>
      <span>{$t("study.focus.title")}</span>
    </a>
    <div class="level-block">
      <span class="level-seed" aria-hidden="true">L</span>
      <div class="level-copy">
        <strong>{xpState ? `Level ${xpState.level}` : $t("study.common.loading")}</strong>
        <span>{xpState ? `${xpState.xp} XP` : "XP"}</span>
      </div>
      <div class="level-meter" aria-hidden="true">
        <div style:width="{xpState?.level_progress_pct ?? 0}%"></div>
      </div>
    </div>
  </header>

  <nav class="focus-rail" aria-label={$t("study.focus.settings_title") as string}>
    <button
      type="button"
      class="rail-btn"
      class:active={panel === "settings"}
      onclick={() => togglePanel("settings")}
      aria-label={$t("study.focus.settings_title") as string}
      title={$t("study.focus.settings_title") as string}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 8.5a3.5 3.5 0 1 1 0 7a3.5 3.5 0 0 1 0-7Z" />
        <path d="M19 13.5v-3l-2.1-.5a7 7 0 0 0-.7-1.6l1.1-1.9-2.1-2.1-1.9 1.1a7 7 0 0 0-1.6-.7L11.2 2H8.3l-.5 2.1a7 7 0 0 0-1.6.7L4.3 3.7 2.2 5.8l1.1 1.9a7 7 0 0 0-.7 1.6L.5 9.8v3l2.1.5a7 7 0 0 0 .7 1.6l-1.1 1.9 2.1 2.1 1.9-1.1a7 7 0 0 0 1.6.7l.5 2.1h2.9l.5-2.1a7 7 0 0 0 1.6-.7l1.9 1.1 2.1-2.1-1.1-1.9a7 7 0 0 0 .7-1.6l2.1-.5Z" />
      </svg>
    </button>
    <button
      type="button"
      class="rail-btn"
      class:active={panel === "history"}
      onclick={() => togglePanel("history")}
      aria-label={$t("study.focus.history") as string}
      title={$t("study.focus.history") as string}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 19V5" />
        <path d="M8 17V9" />
        <path d="M12 15V7" />
        <path d="M16 19V11" />
        <path d="M20 13V5" />
      </svg>
    </button>
    <button
      type="button"
      class="rail-btn"
      class:active={panel === "intent"}
      onclick={() => togglePanel("intent")}
      aria-label={$t("study.focus.intent_label") as string}
      title={$t("study.focus.intent_label") as string}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3v18" />
        <path d="M3 12h18" />
        <circle cx="12" cy="12" r="7" />
      </svg>
    </button>
    <a class="rail-btn" href="/study/progress" aria-label={$t("study.hub.progress") as string} title={$t("study.hub.progress") as string}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 18l5-5 4 4 7-9" />
        <path d="M14 8h6v6" />
      </svg>
    </a>
    <a class="rail-btn" href="/study/music" aria-label={$t("study.hub.music") as string} title={$t("study.hub.music") as string}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M9 18V5l11-2v13" />
        <circle cx="6" cy="18" r="3" />
        <circle cx="17" cy="16" r="3" />
      </svg>
    </a>
    <button
      type="button"
      class="rail-btn exit"
      onclick={exitFullscreen}
      aria-label={$t("study.music.exit") as string}
      title={$t("study.music.exit") as string}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M18 6 6 18" />
        <path d="m6 6 12 12" />
      </svg>
    </button>
  </nav>

  <main class="timer-stage" aria-busy={loading}>
    <div class="mode-tabs" role="tablist" aria-label={$t("study.focus.title") as string}>
      {#each MODES as mode (mode.kind)}
        <button
          type="button"
          role="tab"
          aria-selected={activeKind === mode.kind}
          class:active={activeKind === mode.kind}
          onclick={() => {
            if (!current) selectedKind = mode.kind;
          }}
          disabled={!!current}
        >
          <span>{mode.badge}</span>
          {$t(mode.labelKey)}
        </button>
      {/each}
    </div>

    <button
      type="button"
      class="goal-pill"
      onclick={() => togglePanel("settings")}
      aria-label={$t("study.focus.acc_cycles_title") as string}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="8" />
        <circle cx="12" cy="12" r="3" />
      </svg>
      <span>{$t("study.focus.today_progress")}</span>
      <strong>{history.today_minutes}/{goalMinutes}</strong>
    </button>

    <section class="timer-readout" aria-live="polite">
      {#if displayMode === "standard"}
        <div class="timer-big" class:finished={isFinished}>{timerText}</div>
      {:else}
        <div class="line-display" aria-label={timerText}>
          <div class="line-track">
            <div class="line-fill" style:width="{progressPct}%"></div>
            <div class="line-knob" style:left="{progressPct}%"></div>
          </div>
          <span>{timerText}</span>
        </div>
      {/if}
      <div class="timer-meta">
        <span class="live-dot" aria-hidden="true"></span>
        <span>{current ? $t("study.focus.running") : $t(activeMode.labelKey)}</span>
        {#if targetMinutes > 0}
          <span>{targetMinutes}{$t("study.focus.min")}</span>
        {/if}
      </div>
    </section>

    {#if rpcCustomText.trim()}
      <button type="button" class="intent-pill" onclick={() => togglePanel("intent")}>
        <span>{$t("study.focus.intent_for")}</span>
        <strong>{rpcCustomText}</strong>
      </button>
    {/if}

    <div class="timer-actions">
      {#if current}
        <button type="button" class="primary-action" onclick={() => stop()}>
          {$t(isFinished ? "study.focus.session_ended" : "study.focus.stop")}
        </button>
        <button type="button" class="secondary-action" onclick={() => persistDisplay(displayMode === "standard" ? "line" : "standard")}>
          {displayMode === "standard" ? "Line" : "Timer"}
        </button>
      {:else}
        <button type="button" class="primary-action" disabled={loading} onclick={() => start()}>
          {$t("study.focus.start")}
        </button>
        <button type="button" class="secondary-action" onclick={() => togglePanel("settings")}>
          {$t("study.focus.settings_title")}
        </button>
      {/if}
    </div>
  </main>

  <aside class={`focus-panel ${panel === "settings" ? "settings-open" : ""}`} class:open={panel !== null} aria-hidden={panel === null}>
    <button type="button" class="panel-close" onclick={() => (panel = null)} aria-label={$t("study.common.close") as string}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M18 6 6 18" />
        <path d="m6 6 12 12" />
      </svg>
    </button>

    {#if panel === "settings"}
      <div class="settings-shell">
        <header class="settings-head">
          <div>
            <p class="panel-kicker">Focus</p>
            <h2>{$t("study.focus.settings_title")}</h2>
          </div>
          <span class="save-state">
            {savingState === "saving"
              ? $t("study.focus.settings_saving")
              : savingState === "saved"
                ? $t("study.focus.settings_saved")
                : "Auto-save"}
          </span>
        </header>

        <div class="settings-body">
          <nav class="settings-tabs" aria-label={$t("study.focus.settings_title") as string}>
            {#each SETTINGS_TABS as tab (tab.id)}
              {@const active = settingsTab === tab.id}
              <button
                type="button"
                class="settings-tab"
                class:active
                onclick={() => (settingsTab = tab.id)}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d={tab.icon}></path>
                </svg>
                <span>{$t(tab.labelKey)}</span>
              </button>
            {/each}
          </nav>

          <div class="settings-content">
            {#if settingsTab === "timer"}
              <section class="settings-pane" aria-labelledby="focus-timer-settings">
                <h3 id="focus-timer-settings">{$t("study.focus.acc_cycles_title")}</h3>
                <p class="settings-note">{$t("study.focus.acc_cycles_summary", { focus: settings.pomodoro_minutes, break: settings.short_break_minutes, cycles: settings.cycles_before_long_break })}</p>

                <div class="settings-group">
                  <div class="number-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_cycles_focus")}</strong>
                      <small>{$t("study.focus.preset_pomodoro")}</small>
                    </span>
                    <div class="number-stepper">
                      <button type="button" onclick={() => adjustNumberSetting("pomodoro_minutes", -1, 1, 120)} aria-label="Decrease">-</button>
                      <input type="number" min="1" max="120" bind:value={settings.pomodoro_minutes} oninput={saveSettings} />
                      <button type="button" onclick={() => adjustNumberSetting("pomodoro_minutes", 1, 1, 120)} aria-label="Increase">+</button>
                    </div>
                  </div>

                  <div class="number-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_cycles_short")}</strong>
                      <small>{$t("study.focus.preset_short_break")}</small>
                    </span>
                    <div class="number-stepper">
                      <button type="button" onclick={() => adjustNumberSetting("short_break_minutes", -1, 1, 60)} aria-label="Decrease">-</button>
                      <input type="number" min="1" max="60" bind:value={settings.short_break_minutes} oninput={saveSettings} />
                      <button type="button" onclick={() => adjustNumberSetting("short_break_minutes", 1, 1, 60)} aria-label="Increase">+</button>
                    </div>
                  </div>

                  <div class="number-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_cycles_long")}</strong>
                      <small>{$t("study.focus.preset_long_break")}</small>
                    </span>
                    <div class="number-stepper">
                      <button type="button" onclick={() => adjustNumberSetting("long_break_minutes", -1, 1, 60)} aria-label="Decrease">-</button>
                      <input type="number" min="1" max="60" bind:value={settings.long_break_minutes} oninput={saveSettings} />
                      <button type="button" onclick={() => adjustNumberSetting("long_break_minutes", 1, 1, 60)} aria-label="Increase">+</button>
                    </div>
                  </div>

                  <div class="number-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_cycles_count")}</strong>
                      <small>{$t("study.focus.cycles")}</small>
                    </span>
                    <div class="number-stepper">
                      <button type="button" onclick={() => adjustNumberSetting("cycles_before_long_break", -1, 1, 10)} aria-label="Decrease">-</button>
                      <input type="number" min="1" max="10" bind:value={settings.cycles_before_long_break} oninput={saveSettings} />
                      <button type="button" onclick={() => adjustNumberSetting("cycles_before_long_break", 1, 1, 10)} aria-label="Increase">+</button>
                    </div>
                  </div>
                </div>
              </section>
            {:else if settingsTab === "appearance"}
              <section class="settings-pane" aria-labelledby="focus-appearance-settings">
                <h3 id="focus-appearance-settings">{$t("settings.cat_appearance")}</h3>
                <p class="settings-note">Scene and timer display stay local to this device.</p>

                <div class="settings-group">
                  <div class="option-setting">
                    <span class="setting-copy">
                      <strong>Scene</strong>
                      <small>Background mood</small>
                    </span>
                    <div class="visual-options">
                      {#each SCENES as scene (scene.id)}
                        <button type="button" class:active={sceneId === scene.id} onclick={() => persistScene(scene.id)}>
                          <strong>{scene.label}</strong>
                          <small>{scene.hint}</small>
                        </button>
                      {/each}
                    </div>
                  </div>

                  <div class="option-setting">
                    <span class="setting-copy">
                      <strong>Display</strong>
                      <small>Timer face</small>
                    </span>
                    <div class="visual-options">
                      <button type="button" class:active={displayMode === "standard"} onclick={() => persistDisplay("standard")}>
                        Timer
                      </button>
                      <button type="button" class:active={displayMode === "line"} onclick={() => persistDisplay("line")}>
                        Line
                      </button>
                    </div>
                  </div>
                </div>
              </section>
            {:else if settingsTab === "sounds"}
              <section class="settings-pane" aria-labelledby="focus-sound-settings">
                <h3 id="focus-sound-settings">{$t("study.focus.acc_sounds_title")}</h3>
                <p class="settings-note">
                  Focus Radio is local, instant and made for this screen. Use the music library for real tracks.
                </p>

                <div class="settings-group">
                  <div class="option-setting">
                    <span class="setting-copy">
                      <strong>Focus Radio</strong>
                      <small>{activeMixInfo.subtitle}</small>
                    </span>
                    <div class="visual-options mix-options">
                      {#each FOCUS_MIXES as mix (mix.id)}
                        <button type="button" class:active={activeMix === mix.id} onclick={() => selectFocusMix(mix.id)}>
                          <strong>{mix.label}</strong>
                          <small>{mix.mood}</small>
                        </button>
                      {/each}
                    </div>
                  </div>
                  <label class="range-setting">
                    <span class="setting-copy">
                      <strong>Radio volume</strong>
                      <small>{focusRadioPlaying ? "playing" : "ready"}</small>
                    </span>
                    <div class="range-control">
                      <input
                        type="range"
                        min="0"
                        max="100"
                        value={focusRadioVolume}
                        oninput={(e) => setFocusRadioVolume(Number((e.currentTarget as HTMLInputElement).value))}
                      />
                      <span>{focusRadioVolume}%</span>
                    </div>
                  </label>
                </div>

                <div class="settings-group">
                  <div class="ambient-head">
                    <span class="setting-copy">
                      <strong>Ambient mixer</strong>
                      <small>Rain, cafe, waves and low noise layers</small>
                    </span>
                    <button type="button" class="mini-toggle" class:active={ambientPower} onclick={toggleAmbientPower}>
                      {ambientPower ? "On" : "Off"}
                    </button>
                  </div>
                  {#each AMBIENT_SOUNDS as sound (sound.id)}
                    <label class="range-setting compact">
                      <span class="setting-copy">
                        <strong>{sound.label}</strong>
                        <small>{ambientVolumes[sound.id]}%</small>
                      </span>
                      <div class="range-control">
                        <input
                          type="range"
                          min="0"
                          max="100"
                          value={ambientVolumes[sound.id]}
                          oninput={(e) => setAmbientVolume(sound.id, Number((e.currentTarget as HTMLInputElement).value))}
                        />
                        <span>{ambientVolumes[sound.id]}%</span>
                      </div>
                    </label>
                  {/each}
                </div>

                <div class="settings-group">
                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_sounds_check_notify")}</strong>
                      <small>{settings.notify_end ? $t("study.focus.acc_sounds_notify_on") : $t("study.focus.acc_sounds_notify_off")}</small>
                    </span>
                    <input type="checkbox" bind:checked={settings.notify_end} onchange={saveSettings} />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>

                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_sounds_check_sound")}</strong>
                      <small>{settings.notify_sound ? $t("study.focus.acc_sounds_sound_on", { count: settings.sound_volume }) : $t("study.focus.acc_sounds_sound_off")}</small>
                    </span>
                    <input type="checkbox" bind:checked={settings.notify_sound} onchange={saveSettings} />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>

                  <label class="range-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_sounds_volume", { count: settings.sound_volume })}</strong>
                      <small>{$t("study.focus.notifications.sound_volume")}</small>
                    </span>
                    <div class="range-control">
                      <input type="range" min="0" max="100" bind:value={settings.sound_volume} oninput={saveSettings} disabled={!settings.notify_sound} />
                      <span>{settings.sound_volume}%</span>
                    </div>
                  </label>
                </div>
              </section>
            {:else if settingsTab === "automation"}
              <section class="settings-pane" aria-labelledby="focus-automation-settings">
                <h3 id="focus-automation-settings">{$t("study.focus.acc_breaks_title")}</h3>
                <p class="settings-note">{$t("study.focus.config_autostart")}</p>

                <div class="settings-group">
                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_breaks_check_breaks")}</strong>
                      <small>{settings.auto_start_breaks ? $t("study.focus.acc_breaks_auto_yes") : $t("study.focus.acc_breaks_auto_no")}</small>
                    </span>
                    <input type="checkbox" bind:checked={settings.auto_start_breaks} onchange={saveSettings} />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>

                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_breaks_check_focus")}</strong>
                      <small>{settings.auto_start_focus ? $t("study.focus.acc_breaks_focus_yes") : $t("study.focus.acc_breaks_focus_no")}</small>
                    </span>
                    <input type="checkbox" bind:checked={settings.auto_start_focus} onchange={saveSettings} />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>

                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>{$t("study.focus.acc_breaks_check_player")}</strong>
                      <small>{settings.auto_pause_player ? $t("study.focus.acc_breaks_player_yes") : $t("study.focus.acc_breaks_player_no")}</small>
                    </span>
                    <input type="checkbox" bind:checked={settings.auto_pause_player} onchange={saveSettings} />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>

                  <label class="switch-setting">
                    <span class="setting-copy">
                      <strong>Auto-hide focus UI</strong>
                      <small>{autoHideUi ? "hides while the timer runs" : "always visible"}</small>
                    </span>
                    <input
                      type="checkbox"
                      checked={autoHideUi}
                      onchange={(e) => persistAutoHide((e.currentTarget as HTMLInputElement).checked)}
                    />
                    <span class="switch-visual" aria-hidden="true"></span>
                  </label>
                </div>
              </section>
            {/if}
          </div>
        </div>

        <footer class="settings-footer">
          <button type="button" class="settings-reset" onclick={resetFocusSettings}>Reset</button>
          <button type="button" class="settings-save" disabled>
            {savingState === "saving"
              ? $t("study.focus.settings_saving")
              : savingState === "saved"
                ? $t("study.focus.settings_saved")
                : "Saved"}
          </button>
        </footer>
      </div>
    {:else if panel === "history"}
      <div class="panel-section">
        <p class="panel-kicker">{$t("study.focus.history")}</p>
        <h2>{$t("study.focus.today_session", { count: cyclesToday, min: history.today_minutes })}</h2>
        <div class="stat-row">
          <span>{$t("study.focus.this_week")}</span>
          <strong>{history.week_minutes}{$t("study.focus.min")}</strong>
        </div>
        <div class="stat-row">
          <span>{$t("study.focus.today_progress")}</span>
          <strong>{goalPct}%</strong>
        </div>
      </div>
      <ol class="history-list">
        {#each history.sessions.slice(0, 12) as s (s.id)}
          <li>
            <span>{presetLabel(s.preset)}</span>
            <strong>{s.minutes ?? 0}{$t("study.focus.min")}</strong>
            <small>{formatDate(s.started_at)}</small>
          </li>
        {:else}
          <li class="empty-history">{$t("study.focus.no_history")}</li>
        {/each}
      </ol>
    {:else if panel === "intent"}
      <div class="panel-section">
        <p class="panel-kicker">{$t("study.focus.intent_label")}</p>
        <h2>{$t("study.focus.rpc_default")}</h2>
        <label class="intent-editor">
          <span>{$t("study.focus.intent_prefix")}</span>
          <input
            type="text"
            value={rpcCustomText}
            oninput={(e) => saveRpcText((e.currentTarget as HTMLInputElement).value)}
            placeholder={$t("study.focus.intent_placeholder") as string}
            maxlength="48"
          />
        </label>
        <p class="panel-note">{$t("study.focus.intent_hint")}</p>
      </div>
    {/if}
  </aside>

  {#if panel === null}
  <a class="study-pill" href="/study/notes">
    <span class="study-pill-arrow" aria-hidden="true">‹</span>
    <span>
      <strong>{$t("study.hub.notes")}</strong>
      <small>{$t("study.hub.read")}</small>
    </span>
  </a>
  {/if}

  <div class="music-dock">
    {#if musicPlayer.currentTrack}
      <PlayerBar />
    {:else}
      <div class="focus-radio-dock">
        <div class="radio-now">
          <button
            type="button"
            class="radio-play"
            class:playing={focusRadioPlaying}
            onclick={toggleFocusRadio}
            aria-label={focusRadioPlaying ? "Pause Focus Radio" : "Play Focus Radio"}
          >
            {focusRadioPlaying ? "Pause" : "Play"}
          </button>
          <div>
            <strong>{activeMixInfo.label}</strong>
            <span>{activeMixInfo.subtitle}</span>
          </div>
        </div>

        <div class="radio-mixes" aria-label="Focus radio mixes">
          {#each FOCUS_MIXES as mix (mix.id)}
            <button type="button" class:active={activeMix === mix.id} onclick={() => selectFocusMix(mix.id)}>
              {mix.label}
            </button>
          {/each}
        </div>

        <div class="radio-actions">
          <button type="button" class:active={ambientPower} onclick={toggleAmbientPower}>
            Ambient
          </button>
          <a href="/study/music">Library</a>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .focus-shell {
    --focus-ink: oklch(96% 0.01 85);
    --focus-muted: oklch(78% 0.025 235);
    --focus-panel: oklch(18% 0.025 260 / 0.9);
    --focus-panel-strong: oklch(14% 0.018 260 / 0.985);
    --focus-border: oklch(96% 0.01 85 / 0.14);
    --focus-accent: oklch(73% 0.16 185);
    --focus-warm: oklch(78% 0.14 58);
    --focus-danger: oklch(66% 0.2 25);
    position: fixed;
    inset: 0;
    z-index: 90;
    overflow: hidden;
    color: var(--focus-ink);
    background: oklch(13% 0.03 260);
  }

  .ambient-scene,
  .scene-shade {
    position: absolute;
    inset: 0;
  }

  .ambient-scene {
    background:
      linear-gradient(180deg, oklch(24% 0.055 250), oklch(13% 0.035 260) 58%, oklch(10% 0.018 250)),
      linear-gradient(90deg, oklch(18% 0.06 205), oklch(26% 0.06 35));
  }

  .focus-shell[data-scene="garden"] .ambient-scene {
    background:
      linear-gradient(180deg, oklch(32% 0.08 190), oklch(18% 0.06 155) 56%, oklch(11% 0.03 140)),
      linear-gradient(90deg, oklch(23% 0.08 115), oklch(27% 0.08 65));
  }

  .focus-shell[data-scene="night"] .ambient-scene {
    background:
      linear-gradient(180deg, oklch(16% 0.07 285), oklch(10% 0.04 260) 58%, oklch(8% 0.025 235)),
      linear-gradient(90deg, oklch(12% 0.06 225), oklch(19% 0.07 315));
  }

  .focus-shell[data-scene="library"] .ambient-scene {
    background:
      linear-gradient(180deg, oklch(24% 0.04 45), oklch(13% 0.028 55) 58%, oklch(8% 0.018 65)),
      repeating-linear-gradient(90deg, oklch(34% 0.06 45 / 0.55) 0 2.2rem, oklch(19% 0.04 60 / 0.38) 2.2rem 3rem);
  }

  .focus-shell[data-scene="cafe"] .ambient-scene {
    background:
      linear-gradient(180deg, oklch(31% 0.055 70), oklch(17% 0.045 42) 56%, oklch(10% 0.026 36)),
      radial-gradient(circle at 72% 22%, oklch(78% 0.13 72 / 0.18), transparent 30rem);
  }

  .focus-shell[data-scene="ocean"] .ambient-scene {
    background:
      linear-gradient(180deg, oklch(34% 0.08 215), oklch(18% 0.07 205) 58%, oklch(9% 0.035 218)),
      linear-gradient(110deg, oklch(58% 0.11 185 / 0.18), transparent 52%);
  }

  .focus-shell[data-scene="library"] .scene-window {
    width: min(22rem, 26vw);
    background:
      repeating-linear-gradient(90deg, oklch(72% 0.12 55 / 0.32) 0 1.4rem, transparent 1.4rem 2.1rem),
      linear-gradient(180deg, oklch(31% 0.06 57 / 0.55), oklch(16% 0.04 50 / 0.24));
  }

  .focus-shell[data-scene="cafe"] .scene-lamp {
    border-bottom-color: oklch(80% 0.14 72 / 0.95);
    opacity: 1;
  }

  .focus-shell[data-scene="ocean"] .scene-desk {
    background:
      linear-gradient(180deg, oklch(42% 0.08 205 / 0.72), oklch(15% 0.05 220 / 0.9)),
      repeating-linear-gradient(90deg, transparent 0 7rem, oklch(80% 0.08 190 / 0.08) 7rem 7.5rem);
  }

  .scene-window {
    position: absolute;
    top: 8%;
    left: 11%;
    width: min(32rem, 34vw);
    height: 42vh;
    border: 1px solid oklch(96% 0.02 80 / 0.16);
    border-radius: 8px;
    background:
      linear-gradient(90deg, transparent 49.5%, oklch(96% 0.02 80 / 0.13) 50%, transparent 50.5%),
      linear-gradient(180deg, transparent 49.5%, oklch(96% 0.02 80 / 0.13) 50%, transparent 50.5%),
      linear-gradient(180deg, oklch(55% 0.13 210 / 0.38), oklch(22% 0.08 260 / 0.22));
    box-shadow: 0 34px 90px oklch(8% 0.03 260 / 0.42);
  }

  .scene-shelf {
    position: absolute;
    top: 18%;
    right: 10%;
    width: min(27rem, 29vw);
    height: 12rem;
    border-bottom: 1rem solid oklch(30% 0.06 55 / 0.74);
    background:
      linear-gradient(90deg, oklch(78% 0.13 60 / 0.38) 0 8%, transparent 8% 13%, oklch(62% 0.15 30 / 0.38) 13% 20%, transparent 20% 28%, oklch(72% 0.1 155 / 0.38) 28% 34%, transparent 34%),
      linear-gradient(180deg, transparent 68%, oklch(96% 0.02 80 / 0.08) 69% 72%, transparent 73%);
    opacity: 0.72;
  }

  .scene-desk {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 5rem;
    height: 20vh;
    background:
      linear-gradient(180deg, oklch(36% 0.055 50 / 0.86), oklch(20% 0.045 50 / 0.92)),
      linear-gradient(90deg, transparent, oklch(84% 0.09 70 / 0.12), transparent);
    transform: skewY(-1deg);
    transform-origin: left bottom;
  }

  .scene-lamp {
    position: absolute;
    right: 22%;
    bottom: 22%;
    width: 10rem;
    height: 12rem;
    border-bottom: 1.2rem solid oklch(76% 0.13 66 / 0.82);
    background:
      linear-gradient(90deg, transparent 42%, oklch(75% 0.05 70 / 0.28) 42% 48%, transparent 48%),
      linear-gradient(135deg, transparent 18%, oklch(85% 0.14 70 / 0.2) 18% 48%, transparent 49%);
    opacity: 0.82;
  }

  .scene-shade {
    background:
      linear-gradient(180deg, oklch(0% 0 0 / 0.16), transparent 42%, oklch(0% 0 0 / 0.34)),
      linear-gradient(90deg, oklch(0% 0 0 / 0.22), transparent 24%, transparent 70%, oklch(0% 0 0 / 0.18));
    pointer-events: none;
  }

  .focus-shell[data-ui-idle="1"] .focus-status,
  .focus-shell[data-ui-idle="1"] .focus-rail,
  .focus-shell[data-ui-idle="1"] .mode-tabs,
  .focus-shell[data-ui-idle="1"] .goal-pill,
  .focus-shell[data-ui-idle="1"] .intent-pill,
  .focus-shell[data-ui-idle="1"] .timer-actions,
  .focus-shell[data-ui-idle="1"] .music-dock {
    opacity: 0;
    pointer-events: none;
    transform: translateY(0.35rem);
  }

  .focus-status {
    position: absolute;
    top: calc(1rem + env(safe-area-inset-top));
    left: calc(1rem + env(safe-area-inset-left));
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    width: min(20rem, calc(100vw - 6rem));
    padding: 0.9rem;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: var(--focus-panel-strong);
    box-shadow: 0 18px 48px oklch(0% 0 0 / 0.3);
  }

  .focus-brand,
  .level-block {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    color: var(--focus-ink);
    text-decoration: none;
  }

  .brand-mark,
  .level-seed {
    display: grid;
    place-items: center;
    width: 2.4rem;
    height: 2.4rem;
    flex: 0 0 auto;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: oklch(96% 0.01 85 / 0.08);
    font-weight: 800;
  }

  .level-block {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
  }

  .level-copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 0.1rem;
  }

  .level-copy strong {
    font-size: 0.95rem;
  }

  .level-copy span {
    color: var(--focus-muted);
    font-size: 0.78rem;
  }

  .level-meter {
    grid-column: 1 / -1;
    height: 0.35rem;
    overflow: hidden;
    border-radius: 999px;
    background: oklch(96% 0.01 85 / 0.1);
  }

  .level-meter div {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, var(--focus-accent), oklch(72% 0.16 315));
    transition: width 180ms ease;
  }

  .focus-rail {
    position: absolute;
    top: calc(1rem + env(safe-area-inset-top));
    right: calc(1rem + env(safe-area-inset-right));
    z-index: 3;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .rail-btn {
    display: grid;
    place-items: center;
    width: 2.75rem;
    height: 2.75rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: var(--focus-panel-strong);
    color: var(--focus-ink);
    cursor: pointer;
    transition: transform 140ms ease, border-color 140ms ease, background 140ms ease;
  }

  .rail-btn:hover,
  .rail-btn.active {
    border-color: oklch(83% 0.12 190 / 0.72);
    background: oklch(22% 0.035 245 / 0.96);
    transform: translateY(-1px);
  }

  .rail-btn.exit:hover {
    border-color: var(--focus-danger);
  }

  .rail-btn svg {
    width: 1.05rem;
    height: 1.05rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .timer-stage {
    position: relative;
    z-index: 1;
    display: flex;
    min-height: calc(100vh - 6rem);
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1.25rem;
    padding: 6rem 1.5rem 8.5rem;
  }

  .mode-tabs {
    display: flex;
    gap: 0.35rem;
    padding: 0.35rem;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: oklch(0% 0 0 / 0.24);
  }

  .mode-tabs button {
    min-width: 7.2rem;
    height: 2.55rem;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--focus-muted);
    font: inherit;
    font-size: 0.9rem;
    font-weight: 700;
    cursor: pointer;
    transition: background 140ms ease, color 140ms ease;
  }

  .mode-tabs button span {
    margin-right: 0.45rem;
    color: oklch(80% 0.03 240 / 0.72);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    font-size: 0.72rem;
  }

  .mode-tabs button.active {
    background: var(--focus-ink);
    color: oklch(16% 0.025 255);
  }

  .mode-tabs button:disabled {
    cursor: default;
  }

  .goal-pill,
  .intent-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.55rem;
    min-height: 2.35rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: oklch(0% 0 0 / 0.26);
    color: var(--focus-muted);
    padding: 0 0.95rem;
    font: inherit;
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
  }

  .goal-pill svg {
    width: 0.95rem;
    height: 0.95rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
  }

  .goal-pill strong,
  .intent-pill strong {
    color: var(--focus-ink);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  }

  .timer-readout {
    display: flex;
    min-height: 11rem;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
  }

  .timer-big {
    color: var(--focus-ink);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    font-size: 8.6rem;
    font-weight: 800;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    text-shadow: 0 16px 38px oklch(0% 0 0 / 0.36);
    transition: color 260ms ease, opacity 260ms ease;
  }

  .focus-shell[data-low-time="1"] .timer-big {
    color: var(--focus-warm);
  }

  .timer-big.finished {
    opacity: 0.72;
  }

  .timer-meta {
    display: inline-flex;
    align-items: center;
    gap: 0.7rem;
    color: var(--focus-muted);
    font-size: 0.86rem;
    font-weight: 700;
  }

  .live-dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    background: currentColor;
    box-shadow: 0 0 0 0.35rem oklch(96% 0.01 85 / 0.08);
  }

  .focus-shell[data-active="1"] .live-dot {
    background: var(--focus-accent);
  }

  .line-display {
    display: grid;
    grid-template-columns: minmax(18rem, 34rem) 5rem;
    align-items: center;
    gap: 1rem;
  }

  .line-track {
    position: relative;
    height: 0.35rem;
    border-radius: 999px;
    background: oklch(96% 0.01 85 / 0.26);
  }

  .line-fill {
    height: 100%;
    border-radius: inherit;
    background: var(--focus-ink);
    transition: width 1s linear;
  }

  .line-knob {
    position: absolute;
    top: 50%;
    width: 0.9rem;
    height: 0.9rem;
    border-radius: 999px;
    background: var(--focus-ink);
    transform: translate(-50%, -50%);
    transition: left 1s linear;
  }

  .line-display span {
    color: var(--focus-ink);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    font-size: 1rem;
    font-weight: 800;
  }

  .timer-actions {
    display: flex;
    gap: 0.75rem;
  }

  .primary-action,
  .secondary-action {
    min-width: 8.5rem;
    height: 3rem;
    border-radius: 8px;
    font: inherit;
    font-size: 1rem;
    font-weight: 800;
    cursor: pointer;
    transition: transform 140ms ease, background 140ms ease, border-color 140ms ease;
  }

  .primary-action {
    border: 0;
    background: var(--focus-ink);
    color: oklch(16% 0.025 255);
  }

  .secondary-action {
    border: 1px solid var(--focus-border);
    background: oklch(0% 0 0 / 0.2);
    color: var(--focus-ink);
  }

  .primary-action:hover,
  .secondary-action:hover {
    transform: translateY(-1px);
  }

  .primary-action:disabled {
    cursor: wait;
    opacity: 0.65;
  }

  .focus-panel {
    position: absolute;
    top: calc(1rem + env(safe-area-inset-top));
    right: calc(4.4rem + env(safe-area-inset-right));
    bottom: calc(6.2rem + env(safe-area-inset-bottom));
    z-index: 4;
    width: min(25rem, calc(100vw - 6rem));
    overflow-y: auto;
    padding: 1rem;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: var(--focus-panel-strong);
    box-shadow: 0 24px 80px oklch(0% 0 0 / 0.36);
    transform: translateX(calc(100% + 6rem));
    opacity: 0;
    pointer-events: none;
    transition: transform 220ms cubic-bezier(0.22, 1, 0.36, 1), opacity 160ms ease;
  }

  .focus-panel.settings-open {
    display: flex;
    width: min(43rem, calc(100vw - 6rem));
    overflow: hidden;
    padding: 0;
  }

  .focus-panel.open {
    transform: translateX(0);
    opacity: 1;
    pointer-events: auto;
  }

  .panel-close {
    position: sticky;
    top: 0;
    float: right;
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: oklch(0% 0 0 / 0.28);
    color: var(--focus-ink);
    cursor: pointer;
  }

  .panel-close svg {
    width: 0.9rem;
    height: 0.9rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
  }

  .focus-panel.settings-open > .panel-close {
    position: absolute;
    top: 0.9rem;
    right: 0.9rem;
    z-index: 3;
    float: none;
  }

  .settings-shell {
    display: flex;
    min-height: 0;
    width: 100%;
    flex-direction: column;
  }

  .settings-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    min-height: 4.5rem;
    padding: 1rem 3.5rem 1rem 1.1rem;
    border-bottom: 1px solid var(--focus-border);
  }

  .settings-head h2 {
    margin: 0;
    color: var(--focus-ink);
    font-size: 1.08rem;
  }

  .settings-body {
    display: grid;
    grid-template-columns: 10.25rem minmax(0, 1fr);
    min-height: 0;
    flex: 1;
  }

  .settings-tabs {
    display: flex;
    min-height: 0;
    flex-direction: column;
    gap: 0.1rem;
    overflow-y: auto;
    padding: 0.55rem 0;
    border-right: 1px solid var(--focus-border);
  }

  .settings-tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.65rem;
    width: 100%;
    min-height: 2.85rem;
    border: 0;
    background: transparent;
    color: var(--focus-muted);
    padding: 0 0.95rem;
    font: inherit;
    font-size: 0.82rem;
    font-weight: 800;
    text-align: left;
    cursor: pointer;
    transition: background 140ms ease, color 140ms ease;
  }

  .settings-tab::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0.45rem;
    bottom: 0.45rem;
    width: 2px;
    border-radius: 999px;
    background: transparent;
  }

  .settings-tab:hover,
  .settings-tab.active {
    background: oklch(96% 0.01 85 / 0.06);
    color: var(--focus-ink);
  }

  .settings-tab.active::before {
    background: var(--focus-accent);
  }

  .settings-tab svg {
    width: 1.05rem;
    height: 1.05rem;
    flex: 0 0 auto;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .settings-content {
    min-height: 0;
    overflow-y: auto;
    padding: 1.1rem;
  }

  .settings-pane {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }

  .settings-pane h3 {
    margin: 0;
    color: var(--focus-ink);
    font-size: 1rem;
  }

  .settings-note {
    margin: -0.25rem 0 0;
    color: var(--focus-muted);
    font-size: 0.78rem;
    line-height: 1.45;
  }

  .settings-group {
    overflow: hidden;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: oklch(0% 0 0 / 0.14);
  }

  .number-setting,
  .switch-setting,
  .range-setting,
  .option-setting {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    min-height: 4.2rem;
    padding: 0.85rem 0.95rem;
    border-bottom: 1px solid var(--focus-border);
  }

  .settings-group > :last-child {
    border-bottom: 0;
  }

  .setting-copy {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.15rem;
  }

  .setting-copy strong {
    color: var(--focus-ink);
    font-size: 0.86rem;
  }

  .setting-copy small {
    color: var(--focus-muted);
    font-size: 0.72rem;
    font-weight: 700;
  }

  .number-stepper {
    display: grid;
    grid-template-columns: 1.9rem 4.2rem 1.9rem;
    align-items: center;
    gap: 0.35rem;
    flex: 0 0 auto;
  }

  .number-stepper button,
  .number-stepper input {
    height: 2rem;
    border: 1px solid var(--focus-border);
    border-radius: 6px;
    background: oklch(96% 0.01 85 / 0.08);
    color: var(--focus-ink);
    font: inherit;
    font-weight: 800;
  }

  .number-stepper button {
    cursor: pointer;
  }

  .number-stepper button:hover {
    border-color: var(--focus-accent);
  }

  .number-stepper input {
    width: 100%;
    text-align: center;
    appearance: textfield;
    -moz-appearance: textfield;
  }

  .number-stepper input::-webkit-outer-spin-button,
  .number-stepper input::-webkit-inner-spin-button {
    margin: 0;
    -webkit-appearance: none;
  }

  .switch-setting {
    position: relative;
    cursor: pointer;
  }

  .switch-setting input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }

  .switch-visual {
    position: relative;
    width: 2.4rem;
    height: 1.35rem;
    flex: 0 0 auto;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: oklch(96% 0.01 85 / 0.08);
    transition: background 140ms ease, border-color 140ms ease;
  }

  .switch-visual::after {
    content: "";
    position: absolute;
    top: 50%;
    left: 0.18rem;
    width: 0.9rem;
    height: 0.9rem;
    border-radius: 999px;
    background: var(--focus-muted);
    transform: translateY(-50%);
    transition: transform 140ms ease, background 140ms ease;
  }

  .switch-setting input:checked + .switch-visual {
    border-color: oklch(78% 0.13 185 / 0.75);
    background: oklch(58% 0.13 185 / 0.28);
  }

  .switch-setting input:checked + .switch-visual::after {
    background: var(--focus-ink);
    transform: translate(1.05rem, -50%);
  }

  .range-setting {
    align-items: flex-start;
    grid-template-columns: 1fr;
    flex-direction: column;
  }

  .range-setting.compact {
    min-height: 3.5rem;
    padding-block: 0.65rem;
  }

  .range-control {
    display: grid;
    grid-template-columns: 1fr 3rem;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
  }

  .range-control input {
    accent-color: var(--focus-accent);
  }

  .range-control input:disabled {
    opacity: 0.4;
  }

  .range-control span {
    color: var(--focus-muted);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
    font-size: 0.76rem;
    font-weight: 800;
    text-align: right;
  }

  .option-setting {
    align-items: flex-start;
    grid-template-columns: 1fr;
    flex-direction: column;
  }

  .visual-options {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    width: 100%;
  }

  .visual-options button {
    display: inline-flex;
    flex-direction: column;
    justify-content: center;
    gap: 0.08rem;
    min-height: 2.1rem;
    border: 1px solid var(--focus-border);
    border-radius: 6px;
    background: oklch(96% 0.01 85 / 0.05);
    color: var(--focus-muted);
    font: inherit;
    font-size: 0.78rem;
    font-weight: 800;
    padding: 0 0.75rem;
    cursor: pointer;
  }

  .visual-options button strong {
    color: currentColor;
    font-size: 0.78rem;
  }

  .visual-options button small {
    color: var(--focus-muted);
    font-size: 0.65rem;
    font-weight: 700;
  }

  .visual-options button:hover,
  .visual-options button.active {
    border-color: var(--focus-accent);
    color: var(--focus-ink);
  }

  .mix-options button {
    min-width: 6.4rem;
    align-items: flex-start;
  }

  .ambient-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    min-height: 4rem;
    padding: 0.85rem 0.95rem;
    border-bottom: 1px solid var(--focus-border);
  }

  .mini-toggle {
    min-width: 3.2rem;
    min-height: 2rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: oklch(96% 0.01 85 / 0.07);
    color: var(--focus-muted);
    font: inherit;
    font-size: 0.76rem;
    font-weight: 900;
    cursor: pointer;
  }

  .mini-toggle.active,
  .mini-toggle:hover {
    border-color: var(--focus-accent);
    color: var(--focus-ink);
  }

  .settings-footer {
    display: flex;
    gap: 0.75rem;
    padding: 0.75rem 1rem 1rem;
    border-top: 1px solid var(--focus-border);
  }

  .settings-reset,
  .settings-save {
    min-height: 2.35rem;
    flex: 1;
    border-radius: 8px;
    font: inherit;
    font-size: 0.86rem;
    font-weight: 800;
  }

  .settings-reset {
    border: 1px solid var(--focus-border);
    background: oklch(96% 0.01 85 / 0.07);
    color: var(--focus-ink);
    cursor: pointer;
  }

  .settings-save {
    border: 0;
    background: oklch(96% 0.01 85 / 0.16);
    color: var(--focus-muted);
  }

  .panel-section {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    padding: 0.55rem 0 1.1rem;
    border-bottom: 1px solid var(--focus-border);
  }

  .panel-section:last-child {
    border-bottom: 0;
  }

  .panel-kicker {
    margin: 0;
    color: var(--focus-muted);
    font-size: 0.74rem;
    font-weight: 800;
  }

  .panel-section h2 {
    margin: 0;
    color: var(--focus-ink);
    font-size: 1.15rem;
  }

  .field-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.75rem;
  }

  .field-grid label,
  .range-row,
  .intent-editor {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    color: var(--focus-muted);
    font-size: 0.78rem;
    font-weight: 700;
  }

  .field-grid input,
  .intent-editor input {
    width: 100%;
    height: 2.35rem;
    border: 1px solid var(--focus-border);
    border-radius: 6px;
    background: oklch(0% 0 0 / 0.22);
    color: var(--focus-ink);
    font: inherit;
    padding: 0 0.7rem;
  }

  .field-grid input:focus,
  .intent-editor input:focus {
    border-color: var(--focus-accent);
    outline: none;
  }

  .switch-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    color: var(--focus-ink);
    font-size: 0.88rem;
    font-weight: 700;
  }

  .switch-row input {
    width: 1.05rem;
    height: 1.05rem;
    accent-color: var(--focus-accent);
  }

  .range-row input {
    accent-color: var(--focus-accent);
  }

  .scene-options {
    display: flex;
    gap: 0.45rem;
    flex-wrap: wrap;
  }

  .scene-options button {
    height: 2.15rem;
    border: 1px solid var(--focus-border);
    border-radius: 6px;
    background: transparent;
    color: var(--focus-muted);
    font: inherit;
    font-size: 0.8rem;
    font-weight: 800;
    padding: 0 0.75rem;
    cursor: pointer;
  }

  .scene-options button.active,
  .scene-options button:hover {
    border-color: var(--focus-accent);
    color: var(--focus-ink);
  }

  .save-state,
  .panel-note {
    min-height: 1rem;
    margin: 0;
    color: var(--focus-muted);
    font-size: 0.78rem;
  }

  .stat-row,
  .history-list li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.5rem;
    align-items: center;
  }

  .stat-row {
    color: var(--focus-muted);
    font-size: 0.88rem;
  }

  .stat-row strong {
    color: var(--focus-ink);
    font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  }

  .history-list {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .history-list li {
    padding: 0.7rem;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: oklch(96% 0.01 85 / 0.05);
    color: var(--focus-ink);
  }

  .history-list small {
    grid-column: 1 / -1;
    color: var(--focus-muted);
  }

  .empty-history {
    display: block;
    color: var(--focus-muted);
  }

  .study-pill {
    position: absolute;
    right: calc(1rem + env(safe-area-inset-right));
    bottom: calc(6.4rem + env(safe-area-inset-bottom));
    z-index: 2;
    display: inline-flex;
    align-items: center;
    gap: 0.7rem;
    min-height: 4rem;
    padding: 0.7rem 1rem 0.7rem 0.75rem;
    border: 1px solid var(--focus-border);
    border-radius: 8px;
    background: var(--focus-panel);
    color: var(--focus-ink);
    text-decoration: none;
    transition: opacity 140ms ease, transform 140ms ease;
  }

  .study-pill-arrow {
    color: var(--focus-muted);
    font-size: 1.55rem;
    line-height: 1;
  }

  .study-pill span:last-child {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }

  .study-pill small {
    color: var(--focus-muted);
    font-weight: 700;
  }

  .music-dock {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 5;
    min-height: 5rem;
    border-top: 1px solid var(--focus-border);
    background: oklch(9% 0.018 250 / 0.92);
  }

  .focus-radio-dock {
    display: grid;
    grid-template-columns: minmax(16rem, 1fr) auto minmax(10rem, 1fr);
    align-items: center;
    gap: 1rem;
    min-height: 5rem;
    padding: 0.75rem 1rem;
  }

  .radio-now,
  .radio-actions,
  .radio-mixes {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }

  .radio-now {
    min-width: 0;
  }

  .radio-play {
    min-width: 4.2rem;
    min-height: 2.25rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: var(--focus-ink);
    color: oklch(15% 0.025 255);
    font: inherit;
    font-size: 0.82rem;
    font-weight: 900;
    cursor: pointer;
  }

  .radio-play.playing {
    background: oklch(74% 0.14 185);
  }

  .radio-now div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.1rem;
  }

  .radio-now strong {
    color: var(--focus-ink);
    font-size: 0.88rem;
  }

  .radio-now span {
    color: var(--focus-muted);
    font-size: 0.76rem;
    font-weight: 700;
  }

  .radio-mixes {
    justify-content: center;
    overflow-x: auto;
  }

  .radio-mixes button,
  .radio-actions button,
  .radio-actions a {
    min-height: 2rem;
    border: 1px solid var(--focus-border);
    border-radius: 999px;
    background: oklch(96% 0.01 85 / 0.06);
    color: var(--focus-muted);
    font: inherit;
    font-size: 0.76rem;
    font-weight: 900;
    padding: 0 0.75rem;
    text-decoration: none;
    cursor: pointer;
    white-space: nowrap;
  }

  .radio-mixes button.active,
  .radio-mixes button:hover,
  .radio-actions button.active,
  .radio-actions button:hover,
  .radio-actions a:hover {
    border-color: var(--focus-accent);
    color: var(--focus-ink);
  }

  .radio-actions {
    justify-content: flex-end;
  }

  button:focus-visible,
  a:focus-visible {
    outline: 2px solid var(--focus-accent);
    outline-offset: 2px;
  }

  @media (max-width: 900px) {
    .focus-status {
      width: min(17rem, calc(100vw - 5.5rem));
    }

    .mode-tabs {
      max-width: calc(100vw - 2rem);
      overflow-x: auto;
    }

    .mode-tabs button {
      min-width: 6.8rem;
    }

    .timer-big {
      font-size: 6.2rem;
    }

    .study-pill {
      display: none;
    }

    .focus-radio-dock {
      grid-template-columns: 1fr;
      gap: 0.55rem;
      padding: 0.65rem 0.8rem;
    }

    .radio-now,
    .radio-actions,
    .radio-mixes {
      justify-content: flex-start;
    }

    .radio-mixes {
      order: 3;
      width: 100%;
      padding-bottom: 0.2rem;
    }
  }

  @media (max-width: 640px) {
    .focus-status {
      left: 0.75rem;
      right: 4rem;
      width: auto;
      padding: 0.7rem;
    }

    .focus-brand {
      font-size: 0.9rem;
    }

    .level-block {
      display: none;
    }

    .focus-rail {
      top: auto;
      right: 0.75rem;
      bottom: 10.2rem;
      flex-direction: row;
    }

    .rail-btn {
      width: 2.45rem;
      height: 2.45rem;
    }

    .timer-stage {
      min-height: calc(100vh - 5rem);
      padding: 5rem 0.75rem 9rem;
      gap: 1rem;
    }

    .timer-big {
      font-size: 4.6rem;
    }

    .timer-actions {
      width: 100%;
      justify-content: center;
    }

    .primary-action,
    .secondary-action {
      min-width: 0;
      width: 44%;
    }

    .focus-panel {
      inset: auto 0.75rem 5.7rem 0.75rem;
      width: auto;
      max-height: 58vh;
      transform: translateY(calc(100% + 6rem));
    }

    .focus-panel.settings-open {
      width: auto;
      max-height: 72vh;
    }

    .focus-panel.open {
      transform: translateY(0);
    }

    .settings-body {
      grid-template-columns: 1fr;
    }

    .settings-tabs {
      flex-direction: row;
      overflow-x: auto;
      border-right: 0;
      border-bottom: 1px solid var(--focus-border);
      padding: 0.4rem;
    }

    .settings-tab {
      width: auto;
      min-width: max-content;
      min-height: 2.35rem;
      border-radius: 6px;
      padding: 0 0.75rem;
    }

    .settings-tab::before {
      display: none;
    }

    .settings-content {
      padding: 0.85rem;
    }

    .number-setting,
    .switch-setting {
      align-items: flex-start;
      grid-template-columns: 1fr;
    }

    .field-grid {
      grid-template-columns: 1fr;
    }

    .line-display {
      grid-template-columns: minmax(10rem, 1fr);
      width: min(28rem, calc(100vw - 2rem));
    }

    .music-dock {
      min-height: 6.4rem;
    }

    .radio-now span {
      display: none;
    }

    .radio-actions {
      gap: 0.45rem;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .level-meter div,
    .rail-btn,
    .mode-tabs button,
    .primary-action,
    .secondary-action,
    .focus-panel,
    .line-fill,
    .line-knob {
      transition: none;
    }
  }
</style>
