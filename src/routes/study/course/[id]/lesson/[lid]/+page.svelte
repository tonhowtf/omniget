<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { rpcSetSource, rpcClearSource } from "$lib/rpc";
  import { awardXp, bumpCounter } from "$lib/study-gamification";
  import { onFocusBreakStart } from "$lib/study-focus-bridge";
  import { renderMarkdownSync } from "$lib/study-markdown";
  import { t } from "$lib/i18n";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";
  import PlayerShell from "$lib/study-components/player/PlayerShell.svelte";
  import BingeWatchingToast from "$lib/study-components/player/BingeWatchingToast.svelte";
  import SeekHeatmapPanel from "$lib/study-components/player/SeekHeatmapPanel.svelte";
  import KeyboardHelpOverlay from "$lib/study-components/player/KeyboardHelpOverlay.svelte";
  import AnnotateOverlay from "$lib/study-components/notes/AnnotateOverlay.svelte";
  import {
    studyPlayerContext,
    studyPlayerSeek,
    studySettingsGet,
    studySettingsSave,
    studyDetectSkipGaps,
    type SubtitleTrack,
    type AudioTrack,
    type SkipGaps,
    type ThumbnailSlice,
    type NextLesson as PlayerNextLesson,
    type StudySettings,
  } from "$lib/study-bridge";
  import {
    firePlayerEvent,
    invalidatePlayerEventCache,
  } from "$lib/study-player-events";

  type Lesson = {
    id: number;
    course_id: number;
    module_id: number | null;
    position: number;
    title: string;
    video_path: string;
    subtitle_path: string | null;
    duration_ms: number | null;
    current_seconds: number;
    completed: boolean;
    description_raw: string | null;
    description_format: "md" | "html" | "txt" | null;
  };

  type CourseLesson = {
    id: number;
    position: number;
    title: string;
    video_path: string;
    subtitle_path: string | null;
    duration_ms: number | null;
    current_seconds: number;
    completed: boolean;
  };
  type CourseModule = {
    id: number;
    position: number;
    title: string;
    source_path: string;
    lessons: CourseLesson[];
  };
  type CourseDetail = {
    course: { id: number; title: string };
    modules: CourseModule[];
    loose_lessons: CourseLesson[];
  };

  type Note = {
    id: number;
    body: string;
    timestamp_seconds: number | null;
    created_at: number;
    updated_at: number;
  };

  type NotesV2Block = {
    id: number;
    uuid: string;
    page_id: number;
    parent_id: number | null;
    order_idx: number;
    content: string;
    collapsed: boolean;
    properties: Record<string, unknown>;
    created_at: number;
    updated_at: number;
  };

  type Attachment = {
    id: number;
    name: string;
    file_path: string;
  };

  const courseId = $derived(Number($page.params.id));
  const lessonId = $derived(Number($page.params.lid));

  let lesson = $state<Lesson | null>(null);
  let detail = $state<CourseDetail | null>(null);
  let loading = $state(true);
  let error = $state("");
  let videoSrc = $state("");
  let subtitleSrc = $state("");
  let videoRef = $state<HTMLVideoElement | null>(null);
  let subtitles = $state<SubtitleTrack[]>([]);
  let audioTracks = $state<AudioTrack[]>([]);
  let skipGaps = $state<SkipGaps | null>(null);
  let thumbnailSlices = $state<ThumbnailSlice[]>([]);
  let nextLessonAggregated = $state<PlayerNextLesson | null>(null);
  let playerSettings = $state<StudySettings["player"] | null>(null);
  let selectedSubtitleLang = $state<string | null>(null);
  let selectedAudioLang = $state<string | null>(null);
  let lastSeekFromMs = 0;
  let currentTimeMs = $state(0);
  let bingeShown = $state(false);
  let bingeCancelled = $state(false);
  let annotateOpen = $state(false);
  let progressBarEl = $state<HTMLElement | null>(null);
  let helpOverlayOpen = $state(false);
  let nextLessonPreloadUrl = $state<string | null>(null);
  let preloadTriggered = false;

  let lastSavedAt = 0;
  let saveTimer: number | null = null;
  let markedComplete = $state(false);

  let notes = $state<Note[]>([]);
  let notesLoading = $state(false);
  let noteBody = $state("");
  let noteAtTimestamp = $state(true);
  let editingNoteId = $state<number | null>(null);
  let editingBody = $state("");
  let noteInputRef = $state<HTMLTextAreaElement | null>(null);

  const AUTOPLAY_SECONDS = 5;
  let autoplayRemaining = $state<number | null>(null);
  let autoplayTimer: number | null = null;

  const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2] as const;
  let playbackSpeed = $state(1);
  let theaterMode = $state(false);

  function speedKey(cid: number): string {
    return `study.player.speed.course.${cid}`;
  }

  function loadSpeedForCourse(cid: number): number {
    if (typeof localStorage === "undefined") return 1;
    const raw = localStorage.getItem(speedKey(cid));
    if (!raw) return 1;
    const n = Number(raw);
    return SPEED_OPTIONS.includes(n as (typeof SPEED_OPTIONS)[number]) ? n : 1;
  }

  function persistSpeed(cid: number, value: number) {
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(speedKey(cid), String(value));
    } catch {}
  }

  function applySpeed() {
    if (videoRef) videoRef.playbackRate = playbackSpeed;
  }

  function setSpeed(value: number) {
    playbackSpeed = value;
    applySpeed();
    persistSpeed(courseId, value);
  }

  function bumpSpeed(delta: number) {
    const idx = SPEED_OPTIONS.indexOf(
      playbackSpeed as (typeof SPEED_OPTIONS)[number],
    );
    const fallback = SPEED_OPTIONS.indexOf(1);
    const cur = idx < 0 ? fallback : idx;
    const next = Math.max(0, Math.min(SPEED_OPTIONS.length - 1, cur + delta));
    setSpeed(SPEED_OPTIONS[next]);
  }

  $effect(() => {
    if (!Number.isNaN(courseId)) playbackSpeed = loadSpeedForCourse(courseId);
  });

  let mdCache = $state(new Map<string, string>());
  function renderMd(text: string | null | undefined): string {
    if (!text) return "";
    return renderMarkdownSync(text, mdCache, () => {
      mdCache = new Map(mdCache);
    });
  }

  let attachments = $state<Attachment[]>([]);
  let viewerAttachment = $state<Attachment | null>(null);
  let viewerSrc = $state("");

  type PanelKey = "notes" | "attachments" | "info";
  let activePanel = $state<PanelKey>("notes");
  let videoTick = $state(0);

  const panelOptions = $derived([
    {
      value: "notes",
      label: $t("study.notes.panel_title") + ` · ${notes.length}`,
    },
    {
      value: "attachments",
      label: $t("study.attachments.title", { count: attachments.length }),
    },
    {
      value: "info",
      label: "Info",
    },
  ]);

  const lessonProgressPct = $derived.by(() => {
    void videoTick;
    if (!videoRef || !lesson) return 0;
    const total = videoRef.duration;
    if (!isFinite(total) || total <= 0) {
      if (lesson.duration_ms && lesson.duration_ms > 0) {
        const pct = (lesson.current_seconds * 1000) / lesson.duration_ms;
        return Math.min(100, Math.max(0, pct * 100));
      }
      return 0;
    }
    return Math.min(100, Math.max(0, (videoRef.currentTime / total) * 100));
  });

  const flatLessons = $derived.by(() => {
    if (!detail) return [] as CourseLesson[];
    const out: CourseLesson[] = [];
    for (const m of detail.modules) out.push(...m.lessons);
    out.push(...detail.loose_lessons);
    return out;
  });

  const prevLesson = $derived.by(() => {
    const idx = flatLessons.findIndex((l) => l.id === lessonId);
    if (idx <= 0) return null;
    return flatLessons[idx - 1];
  });

  const nextLesson = $derived.by(() => {
    const idx = flatLessons.findIndex((l) => l.id === lessonId);
    if (idx < 0 || idx >= flatLessons.length - 1) return null;
    return flatLessons[idx + 1];
  });

  function formatTime(sec: number | null | undefined): string {
    if (sec == null || !isFinite(sec) || sec < 0) return "0:00";
    const s = Math.floor(sec);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const r = s % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`;
    }
    return `${m}:${String(r).padStart(2, "0")}`;
  }

  async function loadLesson(id: number) {
    loading = true;
    error = "";
    markedComplete = false;
    try {
      const ctxPromise = studyPlayerContext(id);
      const detailPromise = detail
        ? Promise.resolve(detail)
        : pluginInvoke<CourseDetail>("study", "study:courses:get", { courseId });
      const settingsPromise = playerSettings
        ? Promise.resolve<StudySettings>({ player: playerSettings })
        : studySettingsGet();
      const [ctx, d, settings] = await Promise.all([
        ctxPromise,
        detailPromise,
        settingsPromise,
      ]);
      const lessonRaw = await pluginInvoke<Lesson>("study", "study:lesson:get", { lessonId: id });
      lesson = lessonRaw;
      detail = d;
      playerSettings = settings.player ?? null;
      subtitles = ctx.subtitles ?? [];
      audioTracks = ctx.audio_tracks ?? [];
      skipGaps = ctx.skip_gaps ?? null;
      thumbnailSlices = ctx.thumbnails ?? [];
      nextLessonAggregated = ctx.next_lesson ?? null;
      videoSrc = convertFileSrc(ctx.lesson.video_path);
      subtitleSrc = ctx.lesson.subtitle_path ? convertFileSrc(ctx.lesson.subtitle_path) : "";
      markedComplete = lessonRaw.completed;
      const defaultSubLang = playerSettings?.subtitles_default_lang ?? null;
      const secondarySubLang = playerSettings?.subtitles_secondary_lang ?? null;
      const subMatch =
        subtitles.find((s) => s.lang === defaultSubLang)
        ?? subtitles.find((s) => s.lang === secondarySubLang);
      selectedSubtitleLang =
        subMatch?.lang ?? subtitles.find((s) => s.default)?.lang ?? null;
      const defaultAudioLang = playerSettings?.audio_default_lang ?? null;
      const secondaryAudioLang = playerSettings?.audio_secondary_lang ?? null;
      const audioMatch =
        audioTracks.find((a) => a.lang === defaultAudioLang)
        ?? audioTracks.find((a) => a.lang === secondaryAudioLang);
      selectedAudioLang =
        audioMatch?.lang ?? audioTracks.find((a) => a.default)?.lang ?? null;
      const persistedSpeed =
        ctx.library_state.playback_speed ?? settings.player?.playback_speed ?? 1;
      playbackSpeed = persistedSpeed;
      lastSeekFromMs = Math.round((lessonRaw.current_seconds ?? 0) * 1000);
      await tick();
      if (videoRef && lessonRaw.current_seconds > 0 && lessonRaw.current_seconds < 86400) {
        videoRef.currentTime = lessonRaw.current_seconds;
      }
      applySpeed();
      await loadNotes(id);
      await loadAttachments(id);
      pluginInvoke("study", "study:courses:recents:touch", {
        courseId,
        lessonId: id,
      }).catch(() => {});
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function loadAttachments(id: number) {
    try {
      attachments = await pluginInvoke<Attachment[]>(
        "study",
        "study:lesson:attachments",
        { lessonId: id },
      );
    } catch (e) {
      console.error("loadAttachments failed", e);
      attachments = [];
    }
  }

  function attachmentKind(name: string): "pdf" | "image" | "text" | "code" | "other" {
    const ext = (name.split(".").pop() ?? "").toLowerCase();
    if (ext === "pdf") return "pdf";
    if (["png", "jpg", "jpeg", "gif", "webp", "svg"].includes(ext)) return "image";
    if (["md", "txt", "rtf", "csv"].includes(ext)) return "text";
    if (["json", "yaml", "yml", "toml", "html", "htm", "css", "js", "ts", "py", "rs", "go", "java", "c", "cpp", "h"].includes(ext))
      return "code";
    return "other";
  }

  function attachmentIcon(name: string): string {
    switch (attachmentKind(name)) {
      case "pdf":
        return "📄";
      case "image":
        return "🖼";
      case "text":
      case "code":
        return "📝";
      default:
        return "📎";
    }
  }

  function openAttachment(a: Attachment) {
    const kind = attachmentKind(a.name);
    if (kind === "pdf" || kind === "image") {
      viewerAttachment = a;
      viewerSrc = convertFileSrc(a.file_path);
      if (videoRef && !videoRef.paused) {
        videoRef.pause();
      }
    } else {
      const url = convertFileSrc(a.file_path);
      window.open(url, "_blank", "noopener");
    }
  }

  function closeViewer() {
    viewerAttachment = null;
    viewerSrc = "";
  }

  let lessonPageId = $state<number | null>(null);

  function lessonPageName(lessonRowId: number): string {
    return `course/${courseId}/lesson/${lessonRowId}`;
  }

  async function ensureLessonPage(lessonRowId: number): Promise<number> {
    const pageTitle = lesson?.title ?? `Lesson ${lessonRowId}`;
    const r = await pluginInvoke<{ id: number }>(
      "study",
      "study:notes:pages:ensure",
      { name: lessonPageName(lessonRowId), title: pageTitle },
    );
    return r.id;
  }

  async function loadNotes(id: number) {
    notesLoading = true;
    try {
      const pid = await ensureLessonPage(id);
      lessonPageId = pid;
      const blocks = await pluginInvoke<NotesV2Block[]>(
        "study",
        "study:notes:blocks:list_children",
        { pageId: pid, parentId: null },
      );
      notes = blocks.map((b) => ({
        id: b.id,
        body: b.content,
        timestamp_seconds:
          typeof b.properties?.timestamp === "number"
            ? (b.properties.timestamp as number)
            : null,
        created_at: b.created_at,
        updated_at: b.updated_at,
      }));
      notes.sort((a, b) => {
        const ta = a.timestamp_seconds ?? Number.POSITIVE_INFINITY;
        const tb = b.timestamp_seconds ?? Number.POSITIVE_INFINITY;
        if (ta !== tb) return ta - tb;
        return a.created_at - b.created_at;
      });
    } catch (e) {
      console.error("loadNotes failed", e);
    } finally {
      notesLoading = false;
    }
  }

  let unsubBreak: (() => void) | null = null;

  let unlistenClipHotkey: (() => void) | null = null;

  onMount(() => {
    loadLesson(lessonId);
    window.addEventListener("keydown", onKeyDown);
    unsubBreak = onFocusBreakStart(() => {
      if (videoRef && !videoRef.paused) {
        videoRef.pause();
        void persistProgress(videoRef.currentTime, false);
      }
    });
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlistenClipHotkey = await listen("clip-hotkey-pressed", () => {
          void clipLast60s();
        });
      } catch {
        // event listener optional
      }
    })();
  });

  $effect(() => {
    if (!lesson) return;
    if (lesson.id !== lessonId) {
      bingeShown = false;
      bingeCancelled = false;
      preloadTriggered = false;
      nextLessonPreloadUrl = null;
      invalidatePlayerEventCache(lesson.id);
      loadLesson(lessonId);
    }
  });

  async function persistProgress(seconds: number, completed: boolean) {
    if (!lesson) return;
    try {
      await pluginInvoke("study", "study:progress:mark_lesson", {
        lessonId: lesson.id,
        seconds,
        completed,
      });
    } catch (e) {
      console.error("persistProgress failed", e);
    }
  }

  function onTimeUpdate() {
    if (!videoRef || !lesson) return;
    videoTick++;
    const now = Date.now();
    const cur = videoRef.currentTime;
    const dur = videoRef.duration;
    const curMs = Math.round(cur * 1000);
    currentTimeMs = curMs;
    if (Math.abs(curMs - lastSeekFromMs) < 1500) {
      lastSeekFromMs = curMs;
    }
    const threshold = playerSettings?.completion_threshold ?? 0.95;
    if (isFinite(dur) && dur > 0 && cur / dur >= threshold) {
      if (!markedComplete) {
        markedComplete = true;
        persistProgress(cur, true);
        firePlayerEvent("completed", lesson.id, { time_ms: curMs });
      }
      if (!bingeShown && !bingeCancelled && nextLessonAggregated) {
        bingeShown = true;
      }
    }
    if (
      !preloadTriggered &&
      isFinite(dur) &&
      dur > 0 &&
      cur / dur >= 0.8 &&
      nextLessonAggregated &&
      !nextLessonPreloadUrl
    ) {
      preloadTriggered = true;
      preloadNextLesson(nextLessonAggregated.id);
    }
    if (now - lastSavedAt > 5000) {
      lastSavedAt = now;
      persistProgress(cur, markedComplete);
    }
  }

  function onSeeked() {
    if (!videoRef || !lesson) return;
    const toMs = Math.round(videoRef.currentTime * 1000);
    const fromMs = lastSeekFromMs;
    if (Math.abs(toMs - fromMs) < 1500) {
      lastSeekFromMs = toMs;
      return;
    }
    studyPlayerSeek({ lessonId: lesson.id, fromMs, toMs }).catch(() => {});
    firePlayerEvent("seek", lesson.id, { from_ms: fromMs, to_ms: toMs });
    lastSeekFromMs = toMs;
  }

  function onPlay() {
    if (!lesson) return;
    firePlayerEvent("play", lesson.id, {
      time_ms: Math.round((videoRef?.currentTime ?? 0) * 1000),
    });
  }

  function onPause() {
    if (!lesson) return;
    firePlayerEvent("pause", lesson.id, {
      time_ms: Math.round((videoRef?.currentTime ?? 0) * 1000),
    });
  }

  function onSubtitleChange(lang: string | null) {
    selectedSubtitleLang = lang;
    if (playerSettings) {
      const next: StudySettings = {
        player: { ...playerSettings, subtitles_default_lang: lang ?? "" },
      };
      studySettingsSave(next).catch(() => {});
      playerSettings = next.player ?? null;
    }
  }

  function onAudioChange(lang: string | null) {
    selectedAudioLang = lang;
    if (playerSettings) {
      const next: StudySettings = {
        player: { ...playerSettings, audio_default_lang: lang ?? "" },
      };
      studySettingsSave(next).catch(() => {});
      playerSettings = next.player ?? null;
    }
  }

  const subtitleResolved = $derived.by(() => {
    if (selectedSubtitleLang == null) return null;
    const track = subtitles.find((s) => s.lang === selectedSubtitleLang);
    return track ? convertFileSrc(track.path) : null;
  });

  function onSpeedPickerChange(next: number) {
    setSpeed(next);
    if (playerSettings) {
      const merged: StudySettings = {
        player: { ...playerSettings, playback_speed: next },
      };
      studySettingsSave(merged).catch(() => {});
      playerSettings = merged.player ?? null;
    }
  }

  async function preloadNextLesson(nextId: number) {
    try {
      const next = await pluginInvoke<Lesson>("study", "study:lesson:get", { lessonId: nextId });
      nextLessonPreloadUrl = convertFileSrc(next.video_path);
    } catch (e) {
      console.error("preloadNextLesson failed", e);
    }
  }

  function onSkipGap(toMs: number, kind: "intro" | "outro") {
    if (!videoRef) return;
    videoRef.currentTime = toMs / 1000;
    if (lesson) {
      const eventKind = kind === "intro" ? "skipped_intro" : "skipped_outro";
      firePlayerEvent(eventKind, lesson.id, { to_ms: toMs });
    }
  }

  $effect(() => {
    if (!lesson) return;
    if (skipGaps !== null) return;
    const id = lesson.id;
    studyDetectSkipGaps(id)
      .then((res) => {
        if (lesson?.id === id && res) skipGaps = res;
      })
      .catch(() => {});
  });

  function onBingeCancel() {
    bingeShown = false;
    bingeCancelled = true;
  }

  function onBingeGo() {
    bingeShown = false;
    if (nextLessonAggregated) {
      goto(`/study/course/${courseId}/lesson/${nextLessonAggregated.id}`);
    }
  }

  function onEnded() {
    if (!videoRef) return;
    if (!markedComplete) {
      markedComplete = true;
      persistProgress(videoRef.duration || 0, true);
    }
    if (lesson) {
      firePlayerEvent("ended", lesson.id, {
        duration_ms: Math.round((videoRef.duration ?? 0) * 1000),
      });
    }
    startAutoplayCountdown();
  }

  function startAutoplayCountdown() {
    cancelAutoplay();
    if (!nextLesson) {
      autoplayRemaining = -1;
      return;
    }
    autoplayRemaining = AUTOPLAY_SECONDS;
    autoplayTimer = window.setInterval(() => {
      if (autoplayRemaining == null) return;
      autoplayRemaining -= 1;
      if (autoplayRemaining <= 0) {
        cancelAutoplay();
        if (nextLesson) goToLesson(nextLesson.id);
      }
    }, 1000);
  }

  function cancelAutoplay() {
    if (autoplayTimer != null) {
      clearInterval(autoplayTimer);
      autoplayTimer = null;
    }
    autoplayRemaining = null;
  }

  function playNextNow() {
    if (!nextLesson) return;
    cancelAutoplay();
    goToLesson(nextLesson.id);
  }

  onDestroy(() => {
    if (saveTimer) {
      clearInterval(saveTimer);
      saveTimer = null;
    }
    if (videoRef && lesson && !markedComplete) {
      const cur = videoRef.currentTime;
      if (cur > 1) {
        persistProgress(cur, false);
      }
    }
    cancelAutoplay();
    window.removeEventListener("keydown", onKeyDown);
    unsubBreak?.();
    unsubBreak = null;
    unlistenClipHotkey?.();
    unlistenClipHotkey = null;
    void rpcClearSource("video");
  });

  $effect(() => {
    if (!lesson) return;
    const courseTitle = detail?.course?.title ?? "";
    const durationMs = lesson.duration_ms ?? 0;
    const positionSec = Math.floor(videoRef?.currentTime ?? 0);
    const isPaused = videoRef ? videoRef.paused : true;
    void rpcSetSource({
      source: "video",
      details: lesson.title ?? "Aula",
      state: courseTitle || "—",
      duration: Math.floor(durationMs / 1000),
      position: positionSec,
      paused: isPaused,
    });
  });

  async function markCompleteNow() {
    if (!lesson) return;
    const wasComplete = markedComplete;
    markedComplete = true;
    await persistProgress(videoRef?.currentTime ?? 0, true);
    if (!wasComplete) {
      void awardXp("lesson_completed", 25, {
        lesson_id: lesson.id,
        course_id: courseId,
      });
      void bumpCounter("lessons_completed", 1);
    }
  }

  function goToLesson(id: number) {
    goto(`/study/course/${courseId}/lesson/${id}`);
  }

  function focusNoteInput() {
    if (videoRef && !videoRef.paused) {
      videoRef.pause();
    }
    noteAtTimestamp = true;
    tick().then(() => {
      noteInputRef?.focus();
    });
  }

  let screenshotToast = $state("");
  let clipping = $state(false);
  let clipToast = $state("");

  type ClipResult = {
    output_path: string;
    duration_secs: number;
    size_bytes: number;
    used_reencode: boolean;
  };

  async function clipLast60s() {
    if (!videoRef || !lesson) return;
    if (!videoRef.videoWidth || !videoRef.videoHeight) {
      clipToast = $t("study.lesson.clip_no_video") as string;
      setTimeout(() => (clipToast = ""), 2400);
      return;
    }
    if (clipping) return;
    clipping = true;
    clipToast = $t("study.lesson.clip_loading") as string;
    try {
      const cur = videoRef.currentTime;
      const duration = 60;
      const start = Math.max(0, cur - duration);
      const actual = Math.min(duration, cur);
      const ts = fmtTimestamp(cur);
      const res = await invoke<ClipResult>("clip_video", {
        req: {
          source_path: lesson.video_path,
          start_secs: start,
          duration_secs: actual,
          dest_dir: null,
          label: `${safeForFilename(lesson.title || "aula")}-${ts}`,
          reencode: null,
        },
      });
      const kb = Math.round(res.size_bytes / 1024);
      clipToast = `${$t("study.lesson.clip_done")} (${kb} KB)`;
      setTimeout(() => (clipToast = ""), 3500);
      void awardXp("clip_saved", 2, {
        lesson_id: lesson.id,
        timestamp: Math.floor(cur),
        used_reencode: res.used_reencode,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      clipToast = `${$t("study.lesson.clip_failed")}: ${msg}`;
      setTimeout(() => (clipToast = ""), 4500);
    } finally {
      clipping = false;
    }
  }

  function fmtTimestamp(secs: number): string {
    const total = Math.floor(secs);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    if (h > 0) {
      return `${h}h${String(m).padStart(2, "0")}m${String(s).padStart(2, "0")}s`;
    }
    return `${m}m${String(s).padStart(2, "0")}s`;
  }

  function safeForFilename(s: string): string {
    return s
      .replace(/[\/\\:*?"<>|]/g, "_")
      .replace(/\s+/g, "_")
      .slice(0, 60);
  }

  async function captureScreenshot() {
    if (!videoRef || !lesson) return;
    if (!videoRef.videoWidth || !videoRef.videoHeight) {
      screenshotToast = "Vídeo ainda não carregou";
      setTimeout(() => (screenshotToast = ""), 2400);
      return;
    }
    try {
      const canvas = document.createElement("canvas");
      canvas.width = videoRef.videoWidth;
      canvas.height = videoRef.videoHeight;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("canvas 2d ctx null");
      ctx.drawImage(videoRef, 0, 0, canvas.width, canvas.height);
      const blob: Blob = await new Promise((resolve, reject) => {
        canvas.toBlob(
          (b) => (b ? resolve(b) : reject(new Error("toBlob null"))),
          "image/png",
        );
      });
      const ts = fmtTimestamp(videoRef.currentTime);
      const filename = `${safeForFilename(lesson.title || "aula")}-${ts}.png`;
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      screenshotToast = `Screenshot salva (${ts})`;
      setTimeout(() => (screenshotToast = ""), 2800);
      void awardXp("screenshot", 1, {
        lesson_id: lesson.id,
        timestamp: Math.floor(videoRef.currentTime),
      });
    } catch (e) {
      screenshotToast = `Falhou: ${e instanceof Error ? e.message : String(e)}`;
      setTimeout(() => (screenshotToast = ""), 4000);
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const tag = target?.tagName?.toLowerCase();
    if (tag === "input" || tag === "textarea" || target?.isContentEditable) {
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      focusNoteInput();
      return;
    }
    if (!videoRef) return;
    const longSeekMs = playerSettings?.seek_step_long_ms ?? 10000;
    const shortSeekMs = playerSettings?.seek_step_short_ms ?? 3000;
    const longSeek = longSeekMs / 1000;
    const shortSeek = shortSeekMs / 1000;
    const escExitFullscreen = playerSettings?.esc_exit_fullscreen ?? true;
    if (e.key === " " || e.code === "Space") {
      e.preventDefault();
      if (videoRef.paused) void videoRef.play().catch(() => {});
      else videoRef.pause();
    } else if (e.shiftKey && (e.key === "J" || e.key === "j")) {
      e.preventDefault();
      videoRef.currentTime = Math.max(0, videoRef.currentTime - shortSeek);
    } else if (e.shiftKey && (e.key === "L" || e.key === "l" || e.key === "K" || e.key === "k")) {
      e.preventDefault();
      videoRef.currentTime = Math.min(
        videoRef.duration || videoRef.currentTime + shortSeek,
        videoRef.currentTime + shortSeek,
      );
    } else if (e.key === "ArrowLeft" || e.key === "j" || e.key === "J") {
      e.preventDefault();
      videoRef.currentTime = Math.max(0, videoRef.currentTime - longSeek);
    } else if (
      e.key === "ArrowRight" ||
      e.key === "l" ||
      e.key === "L" ||
      e.key === "k" ||
      e.key === "K"
    ) {
      e.preventDefault();
      videoRef.currentTime = Math.min(
        videoRef.duration || videoRef.currentTime + longSeek,
        videoRef.currentTime + longSeek,
      );
    } else if (e.key === "[" || e.key === "<" || e.key === ",") {
      e.preventDefault();
      bumpSpeed(-1);
    } else if (e.key === "]" || e.key === ">" || e.key === ".") {
      e.preventDefault();
      bumpSpeed(1);
    } else if (e.key === "f" || e.key === "F") {
      e.preventDefault();
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      else videoRef.requestFullscreen().catch(() => {});
    } else if (e.key === "m" || e.key === "M") {
      e.preventDefault();
      videoRef.muted = !videoRef.muted;
    } else if (e.key === "t" || e.key === "T") {
      e.preventDefault();
      theaterMode = !theaterMode;
    } else if (e.key === "Escape") {
      if (document.fullscreenElement) {
        if (escExitFullscreen) {
          e.preventDefault();
          document.exitFullscreen().catch(() => {});
        }
      } else if (escExitFullscreen === false) {
        e.preventDefault();
        goto(`/study/course/${courseId}`);
      }
    } else if (e.key === "c" || e.key === "C") {
      e.preventDefault();
      if (subtitles.length === 0) return;
      const idx = subtitles.findIndex((s) => s.lang === selectedSubtitleLang);
      if (selectedSubtitleLang == null) {
        onSubtitleChange(subtitles[0].lang);
      } else if (idx === subtitles.length - 1) {
        onSubtitleChange(null);
      } else {
        onSubtitleChange(subtitles[idx + 1].lang);
      }
    } else if (e.key === "?" || (e.shiftKey && e.key === "/")) {
      e.preventDefault();
      helpOverlayOpen = !helpOverlayOpen;
    } else if (e.key === "," && videoRef.paused) {
      e.preventDefault();
      videoRef.currentTime = Math.max(0, videoRef.currentTime - 1 / 30);
    } else if (e.key === "." && videoRef.paused) {
      e.preventDefault();
      videoRef.currentTime = Math.min(
        videoRef.duration || videoRef.currentTime + 1 / 30,
        videoRef.currentTime + 1 / 30,
      );
    } else if (/^[0-9]$/.test(e.key) && videoRef.duration) {
      e.preventDefault();
      videoRef.currentTime = (videoRef.duration * Number(e.key)) / 10;
    }
  }

  async function addNote() {
    if (!lesson) return;
    const body = noteBody.trim();
    if (!body) return;
    const ts = noteAtTimestamp && videoRef ? videoRef.currentTime : null;
    try {
      const pid = lessonPageId ?? (await ensureLessonPage(lesson.id));
      const courseRef = `[[course/${courseId}]]`;
      const content = `${body} ${courseRef}`.trim();
      const created = await pluginInvoke<{ id: number }>(
        "study",
        "study:notes:blocks:create",
        { pageId: pid, content },
      );
      if (ts != null) {
        await pluginInvoke("study", "study:notes:blocks:property:set", {
          blockId: created.id,
          key: "timestamp",
          value: ts,
        });
      }
      noteBody = "";
      await loadNotes(lesson.id);
    } catch (e) {
      console.error("addNote failed", e);
    }
  }

  function onNoteKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      addNote();
    } else if (e.key === "Escape") {
      noteBody = "";
      (e.target as HTMLTextAreaElement).blur();
    }
  }

  function startEdit(n: Note) {
    editingNoteId = n.id;
    editingBody = n.body;
  }

  function cancelEdit() {
    editingNoteId = null;
    editingBody = "";
  }

  async function saveEdit() {
    if (editingNoteId == null) return;
    const body = editingBody.trim();
    if (!body) return;
    try {
      await pluginInvoke("study", "study:notes:blocks:update", {
        id: editingNoteId,
        content: body,
      });
      editingNoteId = null;
      editingBody = "";
      if (lesson) await loadNotes(lesson.id);
    } catch (e) {
      console.error("saveEdit failed", e);
    }
  }

  async function deleteNote(n: Note) {
    if (!confirm($t("study.notes.confirm_delete"))) return;
    try {
      await pluginInvoke("study", "study:notes:blocks:delete", { id: n.id });
      if (lesson) await loadNotes(lesson.id);
    } catch (e) {
      console.error("deleteNote failed", e);
    }
  }

  function jumpTo(seconds: number | null) {
    if (seconds == null || !videoRef) return;
    videoRef.currentTime = seconds;
    videoRef.play().catch(() => {});
  }
</script>

<section class="lesson-page" data-theater={theaterMode ? "1" : "0"}>
  <aside class="sidebar">
    <a class="back" href="/study/course/{courseId}">
      ← {$t("study.lesson.back_to_course")}
    </a>
    {#if detail}
      <h3 class="course-title">{detail.course.title}</h3>
      <nav class="tree">
        {#each detail.modules as m (m.id)}
          <div class="mod">
            <header>{String(m.position).padStart(2, "0")}. {m.title}</header>
            <ul>
              {#each m.lessons as l (l.id)}
                <li>
                  <button
                    type="button"
                    class:active={l.id === lessonId}
                    class:done={l.completed}
                    onclick={() => goToLesson(l.id)}
                  >
                    <span class="n">{String(l.position).padStart(2, "0")}</span>
                    <span class="t">{l.title}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {/each}
        {#if detail.loose_lessons.length > 0}
          <div class="mod">
            <ul>
              {#each detail.loose_lessons as l (l.id)}
                <li>
                  <button
                    type="button"
                    class:active={l.id === lessonId}
                    class:done={l.completed}
                    onclick={() => goToLesson(l.id)}
                  >
                    <span class="n">{String(l.position).padStart(2, "0")}</span>
                    <span class="t">{l.title}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </nav>
    {/if}
  </aside>

  <main class="main">
    {#if loading}
      <p class="muted">{$t("study.lesson.loading")}</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if lesson && videoSrc}
      <PlayerShell
        {videoSrc}
        title={lesson.title}
        courseTitle={detail?.course.title ?? ""}
        backHref={`/study/course/${courseId}`}
        durationMs={lesson.duration_ms}
        initialSeconds={lesson.current_seconds}
        initialPlaybackSpeed={playbackSpeed}
        {subtitles}
        {audioTracks}
        {skipGaps}
        thumbnails={thumbnailSlices}
        settings={playerSettings}
        {selectedSubtitleLang}
        {selectedAudioLang}
        {theaterMode}
        onTimeUpdate={onTimeUpdate}
        onSeek={(fromMs, toMs) => {
          if (!lesson) return;
          studyPlayerSeek({ lessonId: lesson.id, fromMs, toMs }).catch(() => {});
          firePlayerEvent("seek", lesson.id, { from_ms: fromMs, to_ms: toMs });
        }}
        onPlay={onPlay}
        onPause={onPause}
        onEnded={onEnded}
        onSubtitleChange={onSubtitleChange}
        onAudioChange={onAudioChange}
        onSpeedChange={onSpeedPickerChange}
        onSkipGap={onSkipGap}
        onTheaterToggle={() => (theaterMode = !theaterMode)}
        onClose={() => goto(`/study/course/${courseId}`)}
        onVideoEl={(el) => (videoRef = el)}
      />
      <SeekHeatmapPanel lessonId={lesson.id} />

      {#if autoplayRemaining != null && !bingeShown}
        <div class="autoplay-banner">
          {#if autoplayRemaining < 0}
            <span>{$t("study.lesson.autoplay_last")}</span>
            <button type="button" class="btn" onclick={cancelAutoplay}>
              ✕
            </button>
          {:else}
            <div class="autoplay-text">
              <strong>{$t("study.lesson.autoplay_title")}:</strong>
              <span>{nextLesson?.title ?? ""}</span>
              <small>{$t("study.lesson.autoplay_in", { seconds: autoplayRemaining })}</small>
            </div>
            <button type="button" class="btn" onclick={cancelAutoplay}>
              {$t("study.lesson.autoplay_cancel")}
            </button>
            <button type="button" class="btn primary" onclick={playNextNow}>
              {$t("study.lesson.autoplay_play_now")} ▶
            </button>
          {/if}
        </div>
      {/if}
      <div class="head">
        <h1>{lesson.title}</h1>
        <div class="ctrls">
          <button
            type="button"
            class="btn"
            disabled={!prevLesson}
            onclick={() => prevLesson && goToLesson(prevLesson.id)}
          >
            ← {$t("study.lesson.prev")}
          </button>
          <button
            type="button"
            class="btn icon-btn"
            onclick={captureScreenshot}
            title="Capturar frame atual como PNG"
            aria-label="Screenshot do frame atual"
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z" />
              <circle cx="12" cy="13" r="4" />
            </svg>
          </button>
          <button
            type="button"
            class="btn icon-btn"
            onclick={clipLast60s}
            disabled={clipping}
            title={$t("study.lesson.clip") as string}
            aria-label={$t("study.lesson.clip_aria") as string}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="6" cy="6" r="3" />
              <circle cx="6" cy="18" r="3" />
              <line x1="20" y1="4" x2="8.12" y2="15.88" />
              <line x1="14.47" y1="14.48" x2="20" y2="20" />
              <line x1="8.12" y1="8.12" x2="12" y2="12" />
            </svg>
          </button>
          <button
            type="button"
            class="btn icon-btn"
            onclick={() => (annotateOpen = !annotateOpen)}
            title="Anotar este momento da aula"
            aria-label="Anotar momento"
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
            </svg>
          </button>
          {#if screenshotToast}
            <span class="screenshot-toast">{screenshotToast}</span>
          {/if}
          {#if clipToast}
            <span class="screenshot-toast">{clipToast}</span>
          {/if}
          <button
            type="button"
            class="btn complete"
            class:done={markedComplete}
            onclick={markCompleteNow}
          >
            {markedComplete ? "✓ " + $t("study.lesson.completed") : $t("study.lesson.mark_complete")}
          </button>
          <button
            type="button"
            class="btn"
            disabled={!nextLesson}
            onclick={() => nextLesson && goToLesson(nextLesson.id)}
          >
            {$t("study.lesson.next")} →
          </button>
        </div>
      </div>

      <nav class="panel-tabs" aria-label="painéis da aula">
        <SegmentedControl
          bind:value={activePanel}
          options={panelOptions}
          ariaLabel="lesson panels"
        />
      </nav>

      {#if activePanel === "attachments"}
        {#if attachments.length > 0}
          <div class="attachments-panel">
            <ul class="attachments-list">
              {#each attachments as a (a.id)}
                <li>
                  <button
                    type="button"
                    class="attachment-chip"
                    onclick={() => openAttachment(a)}
                  >
                    <span class="icon">{attachmentIcon(a.name)}</span>
                    <span class="name">{a.name}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {:else}
          <p class="muted panel-empty">Sem anexos nesta aula.</p>
        {/if}
      {/if}

      {#if activePanel === "info"}
        <div class="info-panel">
          {#if lesson}
            <dl class="info-grid">
              <dt>Aula</dt>
              <dd>{lesson.title}</dd>
              <dt>Posição</dt>
              <dd>#{lesson.position}</dd>
              {#if lesson.duration_ms}
                <dt>Duração</dt>
                <dd>{formatTime(lesson.duration_ms / 1000)}</dd>
              {/if}
              <dt>Status</dt>
              <dd>{markedComplete ? "Completa" : "Em andamento"}</dd>
              {#if videoRef && isFinite(videoRef.duration)}
                <dt>Tempo atual</dt>
                <dd>
                  {formatTime(videoRef.currentTime)} /
                  {formatTime(videoRef.duration)}
                </dd>
              {/if}
            </dl>
            {#if lesson.description_raw && lesson.description_raw.trim()}
              <section class="lesson-description">
                <h3>{$t("study.lesson.description_title")}</h3>
                {#if lesson.description_format === "html"}
                  <div class="md-render">{@html lesson.description_raw}</div>
                {:else if lesson.description_format === "md"}
                  <div class="md-render">{@html renderMd(lesson.description_raw)}</div>
                {:else}
                  <pre class="lesson-description-text">{lesson.description_raw}</pre>
                {/if}
              </section>
            {/if}
          {/if}
        </div>
      {/if}

      <div class="notes-panel" hidden={activePanel !== "notes"}>
        <header class="notes-head">
          <h2>{$t("study.notes.panel_title")}</h2>
          <span class="hint">{$t("study.notes.quick_note_hint")}</span>
        </header>

        <div class="note-composer">
          <textarea
            bind:this={noteInputRef}
            bind:value={noteBody}
            onkeydown={onNoteKeydown}
            placeholder={noteAtTimestamp && videoRef
              ? $t("study.notes.quick_placeholder", {
                  time: formatTime(videoRef.currentTime),
                })
              : $t("study.notes.add_placeholder")}
            rows="3"
          ></textarea>
          <div class="composer-ctrls">
            <label class="ts-toggle">
              <input type="checkbox" bind:checked={noteAtTimestamp} />
              <span>
                {noteAtTimestamp && videoRef
                  ? $t("study.notes.at_time", { time: formatTime(videoRef.currentTime) })
                  : $t("study.notes.no_timestamp")}
              </span>
            </label>
            <button
              type="button"
              class="btn primary"
              disabled={!noteBody.trim()}
              onclick={addNote}
            >
              {$t("study.notes.add")}
            </button>
          </div>
        </div>

        {#if notesLoading}
          <p class="muted">{$t("study.notes.loading")}</p>
        {:else if notes.length === 0}
          <p class="muted">{$t("study.notes.empty")}</p>
        {:else}
          <ul class="notes-list">
            {#each notes as n (n.id)}
              <li class="note">
                {#if editingNoteId === n.id}
                  <textarea bind:value={editingBody} rows="3"></textarea>
                  <div class="note-actions">
                    <button type="button" class="btn" onclick={cancelEdit}>
                      {$t("study.notes.cancel")}
                    </button>
                    <button
                      type="button"
                      class="btn primary"
                      disabled={!editingBody.trim()}
                      onclick={saveEdit}
                    >
                      {$t("study.notes.save")}
                    </button>
                  </div>
                {:else}
                  <div class="note-head">
                    {#if n.timestamp_seconds != null}
                      <button
                        type="button"
                        class="ts-btn"
                        onclick={() => jumpTo(n.timestamp_seconds)}
                        title={$t("study.notes.jump_to", {
                          time: formatTime(n.timestamp_seconds),
                        })}
                      >
                        ▶ {formatTime(n.timestamp_seconds)}
                      </button>
                    {:else}
                      <span class="ts-none">{$t("study.notes.no_timestamp")}</span>
                    {/if}
                    <div class="spacer"></div>
                    <button type="button" class="link" onclick={() => startEdit(n)}>
                      {$t("study.notes.edit")}
                    </button>
                    <button type="button" class="link danger" onclick={() => deleteNote(n)}>
                      {$t("study.notes.delete")}
                    </button>
                  </div>
                  <div class="note-body md-render">{@html renderMd(n.body)}</div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {:else}
      <p class="error">{$t("study.lesson.no_video")}</p>
    {/if}
  </main>
</section>

<KeyboardHelpOverlay open={helpOverlayOpen} onClose={() => (helpOverlayOpen = false)} />

{#if annotateOpen}
  <AnnotateOverlay
    {lessonId}
    getCurrentTime={() => (videoRef ? videoRef.currentTime : 0)}
    onClose={() => (annotateOpen = false)}
  />
{/if}

{#if nextLessonPreloadUrl}
  <link rel="prefetch" href={nextLessonPreloadUrl} as="video" />
{/if}

{#if bingeShown && nextLessonAggregated && !bingeCancelled}
  <BingeWatchingToast
    nextLessonId={nextLessonAggregated.id}
    nextLessonTitle={nextLessonAggregated.title}
    {courseId}
    durationMs={playerSettings?.next_video_notification_ms ?? 5000}
    onCancel={onBingeCancel}
    onGo={onBingeGo}
  />
{/if}

{#if viewerAttachment}
  <div
    class="viewer-backdrop"
    role="presentation"
    onclick={closeViewer}
  >
    <div
      class="viewer"
      role="dialog"
      aria-modal="true"
      onclick={(e) => e.stopPropagation()}
    >
      <header class="viewer-head">
        <span class="viewer-title">
          {attachmentIcon(viewerAttachment.name)}
          {viewerAttachment.name}
        </span>
        <button
          type="button"
          class="viewer-close"
          onclick={closeViewer}
          aria-label="close"
        >
          ✕
        </button>
      </header>
      <div class="viewer-body">
        {#if attachmentKind(viewerAttachment.name) === "image"}
          <img src={viewerSrc} alt={viewerAttachment.name} />
        {:else}
          <iframe
            src={viewerSrc}
            title={viewerAttachment.name}
          ></iframe>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .lesson-page {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    height: 100%;
    min-height: 0;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow-y: auto;
    max-height: calc(100vh - 80px);
  }
  .back {
    color: var(--tertiary);
    text-decoration: none;
    font-size: 12px;
  }
  .back:hover {
    color: var(--secondary);
  }
  .course-title {
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
    margin: 0;
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }
  .mod header {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    padding: 4px 6px;
  }
  .mod ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .mod li button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: transparent;
    border: none;
    padding: 6px 8px;
    border-radius: 6px;
    text-align: left;
    color: var(--secondary);
    font-size: 12px;
    cursor: pointer;
  }
  .mod li button:hover {
    background: var(--sidebar-highlight);
  }
  .mod li button.active {
    background: var(--accent);
    color: var(--on-cta, white);
  }
  .mod li button.done .t {
    color: var(--tertiary);
    text-decoration: line-through;
  }
  .mod li .n {
    font-family: var(--font-mono, monospace);
    color: var(--tertiary);
    font-size: 10px;
    flex: 0 0 auto;
  }
  .mod li .t {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    min-width: 0;
    min-height: 0;
  }
  .player-shell {
    background: black;
    border-radius: var(--border-radius);
    overflow: hidden;
    aspect-ratio: 16 / 9;
    position: relative;
  }
  .player-shell video {
    width: 100%;
    height: 100%;
    display: block;
  }
  .player-toolbar {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    gap: 6px;
    align-items: center;
    background: rgba(0, 0, 0, 0.55);
    border-radius: 6px;
    padding: 4px 6px;
    color: #f5f5f5;
    font-size: 12px;
    backdrop-filter: blur(4px);
    pointer-events: auto;
    z-index: 2;
  }
  .speed-control {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .speed-label {
    font-size: 11px;
    opacity: 0.7;
  }
  .speed-select {
    background: rgba(255, 255, 255, 0.12);
    color: #f5f5f5;
    border: none;
    border-radius: 4px;
    padding: 2px 4px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .speed-select option {
    background: #1a1a1a;
    color: #f5f5f5;
  }
  .theater-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: transparent;
    color: inherit;
    border: none;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 11px;
    cursor: pointer;
  }
  .theater-toggle.active {
    background: rgba(255, 255, 255, 0.18);
  }
  :global(.lesson-page[data-theater="1"] .sidebar),
  :global(.lesson-page[data-theater="1"] .head),
  :global(.lesson-page[data-theater="1"] .lesson-progress),
  :global(.lesson-page[data-theater="1"] .panel-tabs),
  :global(.lesson-page[data-theater="1"] .attachments-panel),
  :global(.lesson-page[data-theater="1"] .info-panel),
  :global(.lesson-page[data-theater="1"] .notes-shell) {
    display: none;
  }
  :global(.lesson-page[data-theater="1"]) {
    grid-template-columns: 1fr;
  }
  :global(.lesson-page[data-theater="1"] .main) {
    padding: 0;
    max-width: min(100vw, 1600px);
    margin: 0 auto;
  }
  .md-render {
    font-size: 13px;
    line-height: 1.6;
    color: var(--secondary);
  }
  :global(.md-render p) { margin: 0 0 8px; }
  :global(.md-render h1, .md-render h2, .md-render h3) {
    font-weight: 600;
    color: var(--secondary);
    margin: 12px 0 6px;
  }
  :global(.md-render h1) { font-size: 16px; }
  :global(.md-render h2) { font-size: 15px; }
  :global(.md-render h3) { font-size: 14px; }
  :global(.md-render code) {
    background: var(--button-elevated);
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  :global(.md-render pre) {
    background: var(--button-elevated);
    padding: 8px;
    border-radius: 5px;
    overflow-x: auto;
    margin: 8px 0;
  }
  :global(.md-render pre code) {
    background: transparent;
    padding: 0;
  }
  :global(.md-render blockquote) {
    border-left: 3px solid var(--accent);
    margin: 8px 0;
    padding: 0 0 0 10px;
    color: var(--tertiary);
  }
  :global(.md-render ul, .md-render ol) {
    margin: 0 0 8px;
    padding-left: 20px;
  }
  :global(.md-render li) { margin: 2px 0; }
  :global(.md-render a) {
    color: var(--accent);
    text-decoration: underline;
  }
  :global(.md-render img) {
    max-width: 100%;
    height: auto;
    border-radius: 4px;
  }
  :global(.md-render hr) {
    border: 0;
    border-top: none;
    margin: 12px 0;
  }
  .lesson-description {
    margin-top: 16px;
    padding-top: 12px;
    border-top: none;
  }
  .lesson-description h3 {
    font-size: 12px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    margin: 0 0 8px;
  }
  .lesson-description-text {
    font-family: inherit;
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    color: var(--secondary);
    margin: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--padding);
    flex-wrap: wrap;
  }
  h1 {
    font-size: 18px;
    font-weight: 500;
    margin: 0;
    color: var(--secondary);
    flex: 1;
    min-width: 200px;
  }
  .ctrls {
    display: flex;
    gap: 6px;
  }
  .btn {
    background: var(--button-elevated);
    border: none;
    color: var(--secondary);
    border-radius: var(--border-radius);
    padding: 6px 12px;
    font-size: 12px;
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--sidebar-highlight);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-cta, white);
    border-color: var(--accent);
  }
  .btn.complete.done {
    background: var(--accent);
    color: var(--on-cta, white);
    border-color: var(--accent);
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
  }
  .error {
    color: var(--error);
    font-size: 14px;
  }

  .autoplay-banner {
    display: flex;
    align-items: center;
    gap: var(--padding);
    padding: 10px 14px;
    border: 1px solid var(--accent);
    border-radius: var(--border-radius);
    background: color-mix(in srgb, var(--accent) 10%, var(--button-elevated));
    flex-wrap: wrap;
  }
  .autoplay-text {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
    color: var(--secondary);
    font-size: 13px;
  }
  .autoplay-text strong {
    color: var(--accent);
  }
  .autoplay-text small {
    color: var(--tertiary);
    font-family: var(--font-mono, monospace);
  }

  .notes-panel {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .notes-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--padding);
    flex-wrap: wrap;
  }
  .notes-head h2 {
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
    margin: 0;
  }
  .notes-head .hint {
    color: var(--tertiary);
    font-size: 11px;
  }
  .note-composer {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .note-composer textarea,
  .note textarea {
    width: 100%;
    min-height: 72px;
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    border-radius: var(--border-radius);
    padding: 8px;
    font-size: 13px;
    font-family: inherit;
    resize: vertical;
  }
  .note-composer textarea:focus,
  .note textarea:focus {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: -1px;
  }
  .composer-ctrls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ts-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--tertiary);
    font-size: 12px;
    cursor: pointer;
  }
  .ts-toggle input {
    accent-color: var(--accent);
  }
  .notes-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--input-bg);
  }
  .note-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .note-head .spacer {
    flex: 1;
  }
  .ts-btn {
    background: var(--accent);
    color: var(--on-cta, white);
    border: none;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 11px;
    font-family: var(--font-mono, monospace);
    cursor: pointer;
  }
  .ts-btn:hover {
    filter: brightness(1.1);
  }
  .ts-none {
    color: var(--tertiary);
    font-size: 11px;
    font-style: italic;
  }
  .link {
    background: transparent;
    border: none;
    color: var(--tertiary);
    font-size: 11px;
    cursor: pointer;
    padding: 2px 4px;
  }
  .link:hover {
    color: var(--secondary);
  }
  .link.danger:hover {
    color: var(--error);
  }
  .note-body {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 13px;
    color: var(--secondary);
    line-height: 1.5;
  }
  .note-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  @media (max-width: 800px) {
    .lesson-page {
      grid-template-columns: 1fr;
    }
    .sidebar {
      max-height: 220px;
    }
  }

  .attachments-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: calc(var(--padding) * 1.5);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .attachments-head h2 {
    font-size: 12px;
    font-weight: 500;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin: 0;
  }
  .attachments-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .attachment-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    font-size: 12px;
    padding: 6px 10px;
    border-radius: var(--border-radius);
    cursor: pointer;
  }
  .attachment-chip:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .attachment-chip .icon {
    font-size: 14px;
  }
  .attachment-chip .name {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .viewer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 1100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px;
  }
  .viewer {
    width: min(1100px, 95vw);
    height: min(90vh, 900px);
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.5);
  }
  .viewer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: none;
  }
  .viewer-title {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .viewer-close {
    background: transparent;
    border: none;
    color: var(--tertiary);
    font-size: 16px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
  }
  .viewer-close:hover {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .viewer-body {
    flex: 1;
    min-height: 0;
    background: var(--bg);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: auto;
  }
  .viewer-body iframe {
    width: 100%;
    height: 100%;
    border: none;
    background: white;
  }
  .viewer-body img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .lesson-progress {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 0 calc(var(--padding) * 0.75);
  }
  .lesson-progress-track {
    flex: 1;
    height: 4px;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-radius: 999px;
    overflow: hidden;
  }
  .lesson-progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease;
  }
  .lesson-progress-fill.complete {
    background: var(--success);
  }
  .lesson-progress-pct {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--tertiary);
    min-width: 36px;
    text-align: right;
  }

  .panel-tabs {
    margin-bottom: calc(var(--padding) * 0.75);
  }
  .panel-empty {
    padding: 20px 0;
    text-align: center;
  }

  .info-panel {
    padding: 12px 0;
  }
  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 0;
    font-size: 13px;
  }
  .info-grid dt {
    color: var(--tertiary);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    align-self: center;
  }
  .info-grid dd {
    margin: 0;
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .lesson-progress-fill {
      transition: none;
    }
  }

  .btn.screenshot {
    padding: 6px 10px;
    font-size: 14px;
    line-height: 1;
  }
  .screenshot-toast {
    font-size: 11px;
    color: var(--success, var(--accent));
    padding: 0 6px;
    align-self: center;
    animation: ss-fade 200ms ease-out;
  }
  @keyframes ss-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .screenshot-toast {
      animation: none;
    }
  }
</style>
