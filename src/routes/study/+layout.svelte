<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { rpcSetSource, rpcClearSource } from "$lib/rpc";
  import { t } from "$lib/i18n";
  import { setToolbar } from "$lib/stores/toolbar-store.svelte";
  import {
    onGamificationToast,
    type GamificationToast,
  } from "$lib/study-gamification";
  import { emitFocusBreakStart } from "$lib/study-focus-bridge";
  import {
    STUDY_NOTES_ENABLED,
    STUDY_ANKI_ENABLED,
    STUDY_FOCUS_ENABLED,
    STUDY_PROGRESS_ENABLED,
    STUDY_ACHIEVEMENTS_ENABLED,
  } from "$lib/study-feature-flags";
  import { beforeNavigate } from "$app/navigation";
  import { musicPlayer, type MusicTrack } from "$lib/study-music/player-store.svelte";
  import { musicUI } from "$lib/study-music/ui-store.svelte";
  import { musicTheme } from "$lib/study-music/theme-store.svelte";
  import { playlistsStore } from "$lib/study-music/playlists-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import PlayerBar from "$lib/study-music-components/PlayerBar.svelte";
  import RightBar from "$lib/study-music-components/RightBar.svelte";
  import AddToPlaylistDialog from "$lib/study-music-components/AddToPlaylistDialog.svelte";
  import RootsManager from "$lib/study-music-components/RootsManager.svelte";
  import EqualizerPanel from "$lib/study-music-components/EqualizerPanel.svelte";
  import SleepTimerPanel from "$lib/study-music-components/SleepTimerPanel.svelte";
  import ListenTogetherPanel from "$lib/study-music-components/ListenTogetherPanel.svelte";
  import LastFmPanel from "$lib/study-music-components/LastFmPanel.svelte";
  import MusicThemePanel from "$lib/study-music-components/MusicThemePanel.svelte";
  import QualityPresetPanel from "$lib/study-music-components/QualityPresetPanel.svelte";
  import SourceOrderPanel from "$lib/study-music-components/SourceOrderPanel.svelte";
  import QobuzPanel from "$lib/study-music-components/QobuzPanel.svelte";
  import LyricsTranslationPanel from "$lib/study-music-components/LyricsTranslationPanel.svelte";
  import SelectionBar from "$lib/study-music-components/SelectionBar.svelte";
  import ContextMenu, { type ContextMenuItem } from "$lib/study-music-components/ContextMenu.svelte";

  const SUBNAV_FULL: { href: string; labelKey: string; icon: string; match: string }[] = [
    { href: "/study", labelKey: "study.hub.title", icon: "M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M9 22V12h6v10", match: "exact" },
    { href: "/study/library", labelKey: "study.hub.library", icon: "M4 6h16M4 12h16M4 18h16", match: "prefix" },
    { href: "/study/anki", labelKey: "study.hub.anki", icon: "M4 5h16v14H4z M4 9h16", match: "prefix" },
    { href: "/study/focus", labelKey: "study.hub.focus", icon: "M12 6v6l4 2 M12 22a10 10 0 1 1 0-20 10 10 0 0 1 0 20z", match: "prefix" },
    { href: "/study/progress", labelKey: "study.hub.progress", icon: "M3 17l6-6 4 4 8-8", match: "prefix" },
    { href: "/study/achievements", labelKey: "study.hub.achievements", icon: "M8 21h8 M12 17v4 M7 4h10v6a5 5 0 0 1-10 0V4z M3 5v3a4 4 0 0 0 4 4 M21 5v3a4 4 0 0 1-4 4", match: "prefix" },
    { href: "/study/notes", labelKey: "study.hub.notes", icon: "M4 4h12l4 4v12H4z M14 4v5h5", match: "prefix" },
    { href: "/study/read", labelKey: "study.hub.read", icon: "M4 4h6a3 3 0 0 1 3 3v13a2 2 0 0 0-2-2H4z M20 4h-6a3 3 0 0 0-3 3v13a2 2 0 0 1 2-2h7z", match: "prefix" },
    { href: "/study/music", labelKey: "study.hub.music", icon: "M9 18V5l12-2v13 M6 18 a3 3 0 1 0 0 0.01z M18 16 a3 3 0 1 0 0 0.01z", match: "prefix" },
    { href: "/study/settings", labelKey: "study.hub.settings", icon: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19 12a7 7 0 1 1-14 0 7 7 0 0 1 14 0z", match: "prefix" },
  ];

  const SUBNAV = SUBNAV_FULL.filter((item) => {
    if (item.href === "/study/notes" && !STUDY_NOTES_ENABLED) return false;
    if (item.href === "/study/anki" && !STUDY_ANKI_ENABLED) return false;
    if (item.href === "/study/focus" && !STUDY_FOCUS_ENABLED) return false;
    if (item.href === "/study/progress" && !STUDY_PROGRESS_ENABLED) return false;
    if (item.href === "/study/achievements" && !STUDY_ACHIEVEMENTS_ENABLED) return false;
    return true;
  });

  function isActive(item: (typeof SUBNAV)[number], path: string): boolean {
    if (item.match === "exact") return path === item.href;
    return path === item.href || path.startsWith(item.href + "/");
  }

  type SearchResult = {
    courses: { id: number; title: string }[];
    lessons: { id: number; title: string; course_id: number; course_title: string }[];
    notes: {
      id: number;
      uuid: string;
      page_id: number;
      page_name: string;
      body: string;
      snippet: string;
      updated_at: number;
    }[];
  };

  let { children } = $props();

  let unlisteners: UnlistenFn[] = [];
  let rescanScheduled = false;

  let paletteOpen = $state(false);
  let term = $state("");
  let results = $state<SearchResult>({
    courses: [],
    lessons: [],
    notes: [],
  });
  let searchTimer: number | null = null;
  let searching = $state(false);
  let paletteInput = $state<HTMLInputElement | null>(null);

  type XpState = {
    xp: number;
    level: number;
    xp_to_next: number;
    level_progress_pct: number;
    updated_at: number;
  };
  let xpState = $state<XpState | null>(null);

  async function refreshXpState() {
    try {
      xpState = await pluginInvoke<XpState>("study", "study:gamification:state");
    } catch (e) {
      console.error("xp state fetch failed", e);
    }
  }

  function focusPresetMinutes(presetId: string): number {
    if (presetId === "stopwatch") return 0;
    const match = presetId.match(/-(\d+)$/);
    if (match) return Math.max(0, Number(match[1]) || 0);
    if (presetId.startsWith("deep")) return 50;
    if (presetId.startsWith("short-break")) return 5;
    if (presetId.startsWith("long-break")) return 15;
    return 25;
  }

  type FocusCurrent = {
    id: number;
    preset: string;
    started_at: string;
    elapsed_minutes: number;
  } | null;

  let focusPoll: number | null = null;
  let lastFiredSessionId: number | null = null;
  let autoPausePlayer = true;

  async function refreshAutoPauseSetting() {
    try {
      const all = await pluginInvoke<{
        focus?: { auto_pause_player?: boolean };
      }>("study", "study:settings:get");
      autoPausePlayer = all?.focus?.auto_pause_player ?? true;
    } catch {}
  }

  function presetLabelStudy(presetId: string): string {
    const labels: Record<string, string> = {
      "pomodoro-25": "Pomodoro 25",
      "deep-50": "Deep Work 50",
      "stopwatch": "Cronômetro",
    };
    return labels[presetId] ?? presetId;
  }

  function presetLabelForRpc(presetId: string): string {
    const minutes = focusPresetMinutes(presetId);
    if (presetId === "stopwatch") return $t("study.focus.preset_custom") as string;
    if (presetId.startsWith("short-break")) {
      return `${$t("study.focus.preset_short_break")} ${minutes}`;
    }
    if (presetId.startsWith("long-break")) {
      return `${$t("study.focus.preset_long_break")} ${minutes}`;
    }
    if (presetId.startsWith("deep")) return `${$t("study.focus.preset_deep")} ${minutes}`;
    return `${$t("study.focus.preset_pomodoro")} ${minutes}`;
  }

  function loadFocusCustomText(): string {
    if (typeof localStorage === "undefined") return "";
    try {
      return localStorage.getItem("study.focus.rpc_text.v1") ?? "";
    } catch {
      return "";
    }
  }

  function pushFocusRpc(cur: FocusCurrent | null) {
    if (!cur) {
      void rpcClearSource("focus");
      return;
    }
    const customText = loadFocusCustomText().trim();
    const target = focusPresetMinutes(cur.preset);
    const startedMs = Date.parse(cur.started_at.replace(" ", "T") + "Z");
    const elapsedSec = Math.max(0, Math.floor((Date.now() - startedMs) / 1000));
    const presetText = presetLabelForRpc(cur.preset);
    const details = customText || $t("study.focus.rpc_default") as string;
    const stateText = customText
      ? (target > 0
          ? `${presetText} · ${Math.floor(elapsedSec / 60)}/${target} min`
          : `${presetText} · ${Math.floor(elapsedSec / 60)} min`)
      : (target > 0
          ? `${presetText} · ${Math.floor(elapsedSec / 60)}/${target} min`
          : `${presetText}`);
    void rpcSetSource({
      source: "focus",
      details,
      state: stateText,
      duration: target > 0 ? target * 60 : 0,
      position: elapsedSec,
      paused: false,
    });
  }

  async function pollFocusForBreak() {
    try {
      const cur = await pluginInvoke<FocusCurrent>("study", "study:focus:current");
      pushFocusRpc(cur);
      if (!cur) return;
      const target = focusPresetMinutes(cur.preset);
      if (target <= 0) return;
      const startedMs = Date.parse(cur.started_at.replace(" ", "T") + "Z");
      const elapsedSec = Math.max(0, Math.floor((Date.now() - startedMs) / 1000));
      if (elapsedSec >= target * 60 && lastFiredSessionId !== cur.id) {
        lastFiredSessionId = cur.id;
        if (autoPausePlayer) {
          emitFocusBreakStart({ sessionId: cur.id, reason: "session_ended" });
        }
      }
    } catch {}
  }

  const totalResults = $derived(
    results.courses.length + results.lessons.length + results.notes.length,
  );

  type PaletteTab = "search" | "jump" | "action";
  let paletteTab = $state<PaletteTab>("search");

  type JumpPage = {
    id: number;
    name: string;
    title: string | null;
    updated_at: number;
  };
  let jumpRecents = $state<JumpPage[]>([]);

  const RECENT_ACTIONS_KEY = "study:palette:recent_actions";
  const MAX_RECENT_ACTIONS = 8;

  function loadRecentActions(): string[] {
    try {
      const raw = localStorage.getItem(RECENT_ACTIONS_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed.filter((s) => typeof s === "string") : [];
    } catch {
      return [];
    }
  }

  function saveRecentAction(id: string) {
    try {
      const current = loadRecentActions().filter((s) => s !== id);
      current.unshift(id);
      const trimmed = current.slice(0, MAX_RECENT_ACTIONS);
      localStorage.setItem(RECENT_ACTIONS_KEY, JSON.stringify(trimmed));
      recentActions = trimmed;
    } catch {}
  }

  let recentActions = $state<string[]>([]);

  type PaletteAction = {
    id: string;
    label: string;
    hint: string;
    icon: string;
    run: () => void | Promise<void>;
  };

  async function loadJumpRecents() {
    if (!STUDY_NOTES_ENABLED) return;
    try {
      const pages = await (
        await import("$lib/notes-bridge")
      ).notesPagesList();
      jumpRecents = pages
        .slice()
        .sort((a, b) => b.updated_at - a.updated_at)
        .slice(0, 10)
        .map((p) => ({
          id: p.id,
          name: p.name,
          title: p.title,
          updated_at: p.updated_at,
        }));
    } catch (e) {
      console.error("loadJumpRecents failed", e);
    }
  }

  async function actionRebuildFts() {
    try {
      await pluginInvoke("study", "study:notes:search:rebuild");
    } catch (e) {
      console.error("rebuild fts failed", e);
    }
  }

  async function actionVacuum() {
    try {
      await pluginInvoke("study", "study:library:vacuum");
    } catch (e) {
      console.error("vacuum failed", e);
    }
  }

  async function actionCreatePage() {
    const name = window.prompt("Nome da nova página:");
    if (!name || !name.trim()) return;
    try {
      const r = await (
        await import("$lib/notes-bridge")
      ).notesPagesCreate({ name: name.trim() });
      void goto(`/study/notes?page=${encodeURIComponent(name.trim())}`);
      void r;
    } catch (e) {
      console.error("create page failed", e);
    }
  }

  async function actionExportGraph() {
    try {
      const { notesExportGraphJson } = await import("$lib/notes-bridge");
      const r = await notesExportGraphJson();
      const blob = new Blob([r.json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `notes-graph-${Date.now()}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      console.error("export graph failed", e);
    }
  }

  function makeActions(): PaletteAction[] {
    const notesActions: PaletteAction[] = STUDY_NOTES_ENABLED
      ? [
          {
            id: "create-page",
            label: $t("study.palette.action_create_page_label"),
            hint: $t("study.palette.action_create_page_hint"),
            icon: "+",
            run: () => {
              closePalette();
              void actionCreatePage();
            },
          },
          {
            id: "open-daily",
            label: $t("study.palette.action_open_daily_label"),
            hint: "Ctrl+J",
            icon: "📅",
            run: () => {
              closePalette();
              void openJournalToday();
            },
          },
          {
            id: "open-templates",
            label: $t("study.palette.action_open_templates_label"),
            hint: "/study/notes/templates",
            icon: "▤",
            run: () => {
              closePalette();
              void goto("/study/notes/templates");
            },
          },
          {
            id: "open-graph",
            label: $t("study.palette.action_open_graph_label"),
            hint: $t("study.palette.action_open_graph_hint"),
            icon: "◈",
            run: () => {
              closePalette();
              void goto("/study/notes/graph");
            },
          },
          {
            id: "rebuild-fts",
            label: $t("study.palette.action_rebuild_fts_label"),
            hint: $t("study.palette.action_rebuild_fts_hint"),
            icon: "↻",
            run: () => {
              closePalette();
              void actionRebuildFts();
            },
          },
          {
            id: "export-graph",
            label: $t("study.palette.action_export_graph_label"),
            hint: $t("study.palette.action_export_graph_hint"),
            icon: "↓",
            run: () => {
              closePalette();
              void actionExportGraph();
            },
          },
        ]
      : [];

    return [
      ...notesActions,
      {
        id: "open-settings",
        label: $t("study.palette.action_open_settings_label"),
        hint: "/study/settings",
        icon: "⚙",
        run: () => {
          closePalette();
          void goto("/study/settings");
        },
      },
      {
        id: "vacuum",
        label: $t("study.palette.action_vacuum_label"),
        hint: $t("study.palette.action_vacuum_hint"),
        icon: "▽",
        run: () => {
          closePalette();
          void actionVacuum();
        },
      },
    ];
  }

  const paletteActions = $derived(makeActions());
  const sortedActions = $derived.by(() => {
    const recent = recentActions;
    const all = paletteActions;
    const byId = new Map(all.map((a) => [a.id, a]));
    const ordered: PaletteAction[] = [];
    for (const id of recent) {
      const a = byId.get(id);
      if (a) {
        ordered.push(a);
        byId.delete(id);
      }
    }
    for (const a of byId.values()) ordered.push(a);
    return ordered;
  });

  function runAction(action: PaletteAction) {
    saveRecentAction(action.id);
    void action.run();
  }

  function scheduleRescan() {
    if (rescanScheduled) return;
    rescanScheduled = true;
    setTimeout(async () => {
      rescanScheduled = false;
      try {
        await pluginInvoke("study", "study:rescan");
      } catch (e) {
        console.error("study rescan failed", e);
      }
    }, 1500);
  }

  function openPalette() {
    paletteOpen = true;
    paletteTab = "search";
    recentActions = loadRecentActions();
    void refreshXpState();
    void loadJumpRecents();
    tick().then(() => paletteInput?.focus());
  }

  function closePalette() {
    paletteOpen = false;
    term = "";
    results = { courses: [], lessons: [], notes: [] };
  }

  async function openJournalToday() {
    if (!STUDY_NOTES_ENABLED) return;
    try {
      const r = await pluginInvoke<{ journal_day: number }>(
        "study",
        "study:notes:journal:today",
      );
      void goto(`/study/notes/journal?day=${r.journal_day}`);
    } catch (e) {
      console.error("openJournalToday failed", e);
    }
  }

  function onGlobalKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      paletteOpen ? closePalette() : openPalette();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && (e.key === "j" || e.key === "J") && !e.shiftKey) {
      e.preventDefault();
      void openJournalToday();
      return;
    }
    if (paletteOpen && e.key === "Escape") {
      e.preventDefault();
      closePalette();
    }
  }

  function runSearch() {
    if (searchTimer != null) clearTimeout(searchTimer);
    const q = term.trim();
    if (!q) {
      results = { courses: [], lessons: [], notes: [] };
      searching = false;
      return;
    }
    searching = true;
    searchTimer = window.setTimeout(async () => {
      try {
        const r = await pluginInvoke<SearchResult>("study", "study:search", {
          term: q,
        });
        results = r;
      } catch (e) {
        console.error("search failed", e);
      } finally {
        searching = false;
      }
    }, 200);
  }

  $effect(() => {
    void term;
    runSearch();
  });

  function goCourse(id: number) {
    closePalette();
    goto(`/study/course/${id}`);
  }
  function goLesson(courseId: number, lessonId: number) {
    closePalette();
    goto(`/study/course/${courseId}/lesson/${lessonId}`);
  }

  let gamificationToasts = $state<{ id: number; toast: GamificationToast }[]>([]);
  let nextToastId = 0;

  function pushToast(toast: GamificationToast) {
    const id = ++nextToastId;
    gamificationToasts = [...gamificationToasts, { id, toast }];
    setTimeout(() => {
      gamificationToasts = gamificationToasts.filter((t) => t.id !== id);
    }, 4000);
    if (toast.kind === "level_up" && paletteOpen) {
      void refreshXpState();
    }
  }

  const ctxItems: ContextMenuItem[] = [
    { id: "play", label: $t("study.music.play") as string, icon: "M8 5v14l11-7z" },
    { id: "queue", label: $t("study.music.add_to_queue") as string, icon: "M3 6h13 M3 12h13 M3 18h9 M17 14v6 M14 17h6" },
    { id: "playlist", label: $t("study.music.add_to_playlist") as string, icon: "M12 5v14 M5 12h14" },
    { id: "favorite", label: $t("study.music.favorite") as string, icon: "M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" },
    { id: "sep1", label: "", separator: true },
    { id: "show", label: $t("study.music.ctx_show_in_folder") as string, icon: "M3 7v13h18V9h-9l-2-2H3z" },
    { id: "lyrics", label: $t("study.music.lyrics_title") as string, icon: "M9 18V5l12-2v13" },
  ];

  function handleCtxAction(id: string) {
    const tr = musicUI.contextMenu.track;
    if (!tr) return;
    if (id === "play") {
      void musicPlayer.play(tr, [tr]);
    } else if (id === "queue") {
      const sel = musicUI.selectedIds.has(tr.id) && musicUI.selectionAnchor
        ? musicUI.selectionAnchor.queue.filter((t) => musicUI.selectedIds.has(t.id))
        : [tr];
      musicPlayer.queue = [...musicPlayer.queue, ...sel];
      showToast("success", $t("study.music.queued_count", { count: sel.length }) as string);
    } else if (id === "playlist") {
      musicUI.openAddToPlaylist(tr);
    } else if (id === "favorite") {
      void musicPlayer.toggleFavorite(tr.id);
    } else if (id === "show") {
      void (async () => {
        try {
          const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
          await revealItemInDir(tr.path);
        } catch {
          showToast("error", $t("study.music.ctx_show_failed") as string);
        }
      })();
    } else if (id === "lyrics") {
      if (musicPlayer.currentTrack?.id !== tr.id) {
        void musicPlayer.play(tr, [tr]);
      }
      musicUI.rightbarTab = "lyrics";
    }
  }

  let lastRpcKey = "";
  let rpcInterval: ReturnType<typeof setInterval> | null = null;
  let lastScrobbleTrackId: number | null = null;
  let lastNowPlayingKey = "";
  let scrobbleTimer: ReturnType<typeof setTimeout> | null = null;

  function pushPresence(track: MusicTrack | null, paused: boolean) {
    if (!track) {
      void rpcClearSource("music");
      lastRpcKey = "";
      return;
    }
    const position = Math.floor(musicPlayer.currentTime);
    const duration = Math.floor((track.duration_ms ?? 0) / 1000);
    const details = track.title ?? track.path;
    const stateParts: string[] = [];
    if (track.artist) stateParts.push(track.artist);
    if (track.album) stateParts.push(track.album);
    const stateStr = stateParts.length > 0 ? stateParts.join(" · ") : "—";
    const sourceTag = track.source === "spotify" ? "Spotify" : "omniget";
    void rpcSetSource({
      source: "music",
      details,
      state: `${stateStr} · ${sourceTag}`,
      duration,
      position,
      paused,
    });
  }

  $effect(() => {
    const tr = musicPlayer.currentTrack;
    const paused = musicPlayer.paused;
    const key = `${tr?.id ?? "none"}:${paused ? "p" : "play"}`;
    if (key !== lastRpcKey) {
      lastRpcKey = key;
      pushPresence(tr, paused);
    }
  });

  $effect(() => {
    const tr = musicPlayer.currentTrack;
    if (!tr || musicPlayer.paused) return;
    if (tr.source === "spotify") return;
    const key = `${tr.id}:np`;
    if (key === lastNowPlayingKey) return;
    lastNowPlayingKey = key;
    void pluginInvoke("study", "study:music:lastfm:now_playing", {
      title: tr.title ?? "",
      artist: tr.artist ?? "",
      album: tr.album ?? "",
      duration: Math.floor((tr.duration_ms ?? 0) / 1000),
    }).catch(() => {});

    if (scrobbleTimer) {
      clearTimeout(scrobbleTimer);
      scrobbleTimer = null;
    }
    if (lastScrobbleTrackId === tr.id) return;
    const minPlay = Math.max(30, Math.min(240, Math.floor((tr.duration_ms ?? 0) / 1000 / 2)));
    scrobbleTimer = setTimeout(() => {
      const cur = musicPlayer.currentTrack;
      if (cur && cur.id === tr.id && !musicPlayer.paused) {
        lastScrobbleTrackId = cur.id;
        void pluginInvoke("study", "study:music:lastfm:scrobble", {
          title: cur.title ?? "",
          artist: cur.artist ?? "",
          album: cur.album ?? "",
          duration: Math.floor((cur.duration_ms ?? 0) / 1000),
        }).catch(() => {});
      }
    }, minPlay * 1000);
  });

  onMount(() => {
    const events = [
      "download-complete",
      "udemy-download-complete",
      "thinkific-download-complete",
      "teachable-download-complete",
      "generic-download-complete",
    ];
    Promise.all(
      events.map((name) => listen(name, () => scheduleRescan())),
    ).then((fns) => {
      unlisteners = fns;
    });
    window.addEventListener("keydown", onGlobalKey);
    const unsubGamif = onGamificationToast(pushToast);
    void refreshAutoPauseSetting();
    void pollFocusForBreak();
    focusPoll = window.setInterval(() => {
      void pollFocusForBreak();
    }, 10_000);
    void playlistsStore.load();
    musicTheme.load();
    rpcInterval = setInterval(() => {
      const tr = musicPlayer.currentTrack;
      if (tr && !musicPlayer.paused) {
        pushPresence(tr, false);
      }
    }, 30_000);
    return () => {
      for (const fn of unlisteners) fn();
      unlisteners = [];
      window.removeEventListener("keydown", onGlobalKey);
      unsubGamif();
      if (focusPoll != null) {
        clearInterval(focusPoll);
        focusPoll = null;
      }
      if (rpcInterval) clearInterval(rpcInterval);
      if (scrobbleTimer) clearTimeout(scrobbleTimer);
      void rpcClearSource("music");
      void rpcClearSource("focus");
    };
  });

  onDestroy(() => {
    if (searchTimer != null) clearTimeout(searchTimer);
  });

  beforeNavigate(({ to, from }) => {
    if (!to) return;
    const path = to.url.pathname;
    if (
      path.startsWith("/study/watch") ||
      path.startsWith("/study/course/") ||
      path.startsWith("/study/anki/study")
    ) {
      musicPlayer.pause();
    }
    if (path.startsWith("/study/music") && from && !from.url.pathname.startsWith("/study/music")) {
      try {
        sessionStorage.setItem(
          "study.music.return_url",
          `${from.url.pathname}${from.url.search}`,
        );
      } catch {
        /* ignore */
      }
    }
  });

  // The study sections live in the window toolbar (segmented control), like
  // Finder's view switcher. The in-page nav below stays as the narrow-window
  // fallback and for keyboard users.
  $effect(() => {
    const path = $page.url.pathname;
    const active = SUBNAV.find((item) => isActive(item, path));
    return setToolbar({
      segments: SUBNAV.map((item) => ({ id: item.href, label: $t(item.labelKey) as string })),
      activeSegment: active?.href,
      onSegment: (href) => { void goto(href); },
    });
  });
</script>

<nav class="subnav" aria-label="study sections">
  {#each SUBNAV as item (item.href)}
    {@const active = isActive(item, $page.url.pathname)}
    <a
      href={item.href}
      class="subnav-tab"
      class:active
      aria-current={active ? "page" : undefined}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d={item.icon}></path>
      </svg>
      <span>{$t(item.labelKey)}</span>
    </a>
  {/each}
</nav>

{@render children()}

{#if musicPlayer.currentTrack && !$page.url.pathname.startsWith("/study/watch") && !$page.url.pathname.startsWith("/study/course/") && !$page.url.pathname.startsWith("/study/anki/study") && !$page.url.pathname.startsWith("/study/music")}
  <div class="global-player-bar">
    <PlayerBar />
  </div>
{/if}

<AddToPlaylistDialog
  open={musicUI.addToPlaylistTrack !== null}
  trackId={musicUI.addToPlaylistTrack?.id ?? null}
  trackName={musicUI.addToPlaylistTrack?.title ?? musicUI.addToPlaylistTrack?.path ?? null}
  onClose={() => musicUI.closeAddToPlaylist()}
/>

<RootsManager
  open={musicUI.rootsOpen}
  onClose={() => musicUI.closeRoots()}
/>

<EqualizerPanel
  open={musicUI.equalizerOpen}
  onClose={() => musicUI.closeEqualizer()}
/>

<SleepTimerPanel
  open={musicUI.sleepTimerOpen}
  onClose={() => musicUI.closeSleepTimer()}
/>

<ListenTogetherPanel
  open={musicUI.listenTogetherOpen}
  onClose={() => musicUI.closeListenTogether()}
/>

<LastFmPanel
  open={musicUI.lastfmOpen}
  onClose={() => musicUI.closeLastFm()}
/>

<MusicThemePanel
  open={musicUI.themeOpen}
  onClose={() => musicUI.closeTheme()}
/>

<LyricsTranslationPanel
  open={musicUI.translationSettingsOpen}
  onClose={() => musicUI.closeTranslationSettings()}
/>

<QualityPresetPanel
  open={musicUI.qualityOpen}
  onClose={() => musicUI.closeQuality()}
/>

<SourceOrderPanel
  open={musicUI.sourcesOpen}
  onClose={() => musicUI.closeSources()}
/>

<QobuzPanel
  open={musicUI.qobuzOpen}
  onClose={() => musicUI.closeQobuz()}
/>

<RightBar />

<SelectionBar />

<ContextMenu
  open={musicUI.contextMenu.open}
  x={musicUI.contextMenu.x}
  y={musicUI.contextMenu.y}
  items={ctxItems}
  onAction={handleCtxAction}
  onClose={() => musicUI.closeContextMenu()}
/>

{#if gamificationToasts.length > 0}
  <div class="gamif-stack" role="status" aria-live="polite">
    {#each gamificationToasts as item (item.id)}
      <div class="gamif-toast" data-kind={item.toast.kind}>
        <span class="gamif-icon">
          {item.toast.kind === "level_up" ? "⬆️" : "🏆"}
        </span>
        <span class="gamif-text">{item.toast.text}</span>
      </div>
    {/each}
  </div>
{/if}

{#if paletteOpen}
  <div
    class="palette-backdrop"
    role="presentation"
    onclick={closePalette}
    onkeydown={(e) => { if (e.key === "Escape") closePalette(); }}
  >
    <div
      class="palette"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); closePalette(); } }}
    >
      <div class="palette-tabs" role="tablist" aria-label={$t("study.palette.aria_modes")}>
        <button
          type="button"
          class="palette-tab"
          class:active={paletteTab === "search"}
          role="tab"
          aria-selected={paletteTab === "search"}
          onclick={() => (paletteTab = "search")}
        >{$t("study.palette.tab_search")}</button>
        {#if STUDY_NOTES_ENABLED}
          <button
            type="button"
            class="palette-tab"
            class:active={paletteTab === "jump"}
            role="tab"
            aria-selected={paletteTab === "jump"}
            onclick={() => (paletteTab = "jump")}
          >{$t("study.palette.tab_jump")}</button>
        {/if}
        <button
          type="button"
          class="palette-tab"
          class:active={paletteTab === "action"}
          role="tab"
          aria-selected={paletteTab === "action"}
          onclick={() => (paletteTab = "action")}
        >{$t("study.palette.tab_action")}</button>
      </div>

      {#if paletteTab === "search"}
        <input
          bind:this={paletteInput}
          type="search"
          bind:value={term}
          placeholder={$t("study.search.placeholder")}
          autocomplete="off"
        />
        <div class="palette-body">
          {#if term.trim() === ""}
            <p class="muted">{$t("study.search.hint")}</p>
          {:else if searching}
            <p class="muted">{$t("study.common.loading")}</p>
          {:else if totalResults === 0}
            <p class="muted">{$t("study.search.no_results", { term })}</p>
          {:else}
            {#if results.courses.length > 0}
              <section>
                <h4>{$t("study.search.courses")}</h4>
                <ul>
                  {#each results.courses as c (c.id)}
                    <li>
                      <button type="button" onclick={() => goCourse(c.id)}>
                        📚 {c.title}
                      </button>
                    </li>
                  {/each}
                </ul>
              </section>
            {/if}
            {#if results.lessons.length > 0}
              <section>
                <h4>{$t("study.search.lessons")}</h4>
                <ul>
                  {#each results.lessons as l (l.id)}
                    <li>
                      <button
                        type="button"
                        onclick={() => goLesson(l.course_id, l.id)}
                      >
                        ▶ {l.title}
                        <small>{l.course_title}</small>
                      </button>
                    </li>
                  {/each}
                </ul>
              </section>
            {/if}
            {#if STUDY_NOTES_ENABLED && results.notes.length > 0}
              <section>
                <h4>{$t("study.search.notes")}</h4>
                <ul>
                  {#each results.notes as n (n.id)}
                    <li>
                      <button
                        type="button"
                        onclick={() => {
                          closePalette();
                          goto(
                            `/study/notes?page=${encodeURIComponent(n.page_name)}`,
                          );
                        }}
                      >
                        ✏ {n.snippet || n.body.slice(0, 80)}
                        <small>{n.page_name}</small>
                      </button>
                    </li>
                  {/each}
                </ul>
              </section>
            {/if}
          {/if}
        </div>
      {:else if paletteTab === "jump"}
        <div class="palette-body">
          <section>
            <h4>{$t("study.palette.heading_shortcuts")}</h4>
            <ul>
              <li>
                <button
                  type="button"
                  onclick={() => {
                    closePalette();
                    void openJournalToday();
                  }}
                >
                  📅 {$t("study.palette.shortcut_daily")}
                  <small>Ctrl+J</small>
                </button>
              </li>
              <li>
                <button
                  type="button"
                  onclick={() => {
                    closePalette();
                    void goto("/study/notes/journal");
                  }}
                >
                  📆 {$t("study.palette.shortcut_journal_history")}
                </button>
              </li>
              <li>
                <button
                  type="button"
                  onclick={() => {
                    closePalette();
                    void goto("/study/notes/graph");
                  }}
                >
                  ◈ {$t("study.palette.shortcut_graph")}
                </button>
              </li>
              <li>
                <button
                  type="button"
                  onclick={() => {
                    closePalette();
                    void goto("/study/notes/templates");
                  }}
                >
                  ▤ {$t("study.palette.shortcut_templates")}
                </button>
              </li>
            </ul>
          </section>
          <section>
            <h4>{$t("study.palette.heading_recent_pages")}</h4>
            {#if jumpRecents.length === 0}
              <p class="muted">{$t("study.palette.empty_pages")}</p>
            {:else}
              <ul>
                {#each jumpRecents as p (p.id)}
                  <li>
                    <button
                      type="button"
                      onclick={() => {
                        closePalette();
                        void goto(`/study/notes?page=${encodeURIComponent(p.name)}`);
                      }}
                    >
                      ✏ {p.title ?? p.name}
                      <small>{p.name}</small>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        </div>
      {:else}
        <div class="palette-body">
          <section>
            <h4>{$t("study.palette.heading_actions")}</h4>
            <ul>
              {#each sortedActions as a (a.id)}
                <li>
                  <button type="button" onclick={() => runAction(a)}>
                    <span class="palette-action-icon">{a.icon}</span>
                    {a.label}
                    <small>{a.hint}</small>
                  </button>
                </li>
              {/each}
            </ul>
          </section>
        </div>
      {/if}
      {#if xpState}
        <footer class="palette-footer">
          <span class="palette-level">L{xpState.level}</span>
          <div
            class="palette-bar"
            role="progressbar"
            aria-valuenow={xpState.level_progress_pct}
            aria-valuemin="0"
            aria-valuemax="100"
            aria-label={$t("study.palette.aria_xp_progress", { pct: xpState.level_progress_pct, level: xpState.level + 1 })}
          >
            <div
              class="palette-bar-fill"
              style:width="{xpState.level_progress_pct}%"
            ></div>
          </div>
          <span class="palette-hint"
            >{xpState.level_progress_pct}% até L{xpState.level + 1} ·
            {xpState.xp_to_next} XP</span
          >
        </footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    z-index: 1000;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 80px;
  }
  .palette {
    width: min(600px, 90vw);
    max-height: 70vh;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .palette input {
    background: transparent;
    border: none;
    border-bottom: none;
    color: var(--secondary);
    padding: 14px 16px;
    font-size: 15px;
    outline: none;
  }
  .palette-tabs {
    display: flex;
    gap: 4px;
    padding: 6px 6px 0;
    border-bottom: none;
  }
  .palette-tab {
    padding: 6px 12px;
    border: 0;
    border-radius: var(--border-radius) var(--border-radius) 0 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: color 120ms ease, border-color 120ms ease;
  }
  .palette-tab:hover {
    color: var(--text);
  }
  .palette-tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .palette-action-icon {
    display: inline-block;
    width: 18px;
    text-align: center;
    margin-right: 6px;
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .palette-body {
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .palette-body section h4 {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    margin: 6px 10px 4px;
  }
  .palette-body ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .palette-body li button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: transparent;
    border: none;
    padding: 8px 10px;
    border-radius: 6px;
    text-align: left;
    color: var(--secondary);
    font-size: 13px;
    cursor: pointer;
  }
  .palette-body li button:hover {
    background: var(--sidebar-highlight);
  }
  .palette-body li button small {
    color: var(--tertiary);
    font-size: 11px;
    margin-left: auto;
  }
  .muted {
    color: var(--tertiary);
    font-size: 13px;
    padding: 10px;
    text-align: center;
  }

  .palette-footer {
    display: flex;
    align-items: center;
    gap: 10px;
    border-top: none;
    padding: 8px 14px;
    font-size: 11px;
    color: var(--tertiary);
  }
  .palette-level {
    font-weight: 600;
    color: var(--secondary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .palette-bar {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: var(--input-border);
    overflow: hidden;
  }
  .palette-bar-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }
  @media (prefers-reduced-motion: reduce) {
    .palette-bar-fill {
      transition: none;
    }
  }
  .palette-hint {
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .subnav {
    display: none;
    gap: 2px;
    padding: var(--space-3) var(--space-4) 0;
    margin: 0 auto;
    max-width: 960px;
    width: 100%;
    overflow-x: auto;
    scrollbar-width: none;
    -ms-overflow-style: none;
    border-bottom: none;
  }

  /* narrow windows: the toolbar hides its centre, so the tabs come back here */
  @media (max-width: 980px) {
    .subnav {
      display: flex;
    }
  }
  .subnav::-webkit-scrollbar {
    display: none;
  }
  .subnav-tab {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.85rem;
    color: var(--tertiary);
    text-decoration: none;
    font-size: 13px;
    font-weight: 500;
    border-radius: var(--border-radius);
    white-space: nowrap;
    transition:
      color 150ms ease,
      background 150ms ease;
  }
  .subnav-tab:hover {
    color: var(--secondary);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .subnav-tab.active {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .subnav-tab svg {
    flex-shrink: 0;
  }

  .gamif-stack {
    position: fixed;
    bottom: 20px;
    right: 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 200;
    pointer-events: none;
  }
  .gamif-toast {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-radius: 10px;
    border: 1px solid color-mix(in oklab, var(--accent) 35%, transparent);
    background: var(--surface, var(--bg-elevated, var(--bg)));
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    box-shadow: 0 8px 24px color-mix(in oklab, black 22%, transparent);
    animation: gamif-slide 240ms ease-out;
    pointer-events: auto;
  }
  .gamif-toast[data-kind="level_up"] {
    border-color: var(--success, var(--accent));
    background: color-mix(in oklab, var(--success, var(--accent)) 10%, var(--surface, var(--bg)));
  }
  .gamif-toast[data-kind="achievement"] {
    border-color: var(--warning, var(--accent));
    background: color-mix(in oklab, var(--warning, var(--accent)) 10%, var(--surface, var(--bg)));
  }
  .gamif-icon {
    font-size: 18px;
  }
  @keyframes gamif-slide {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .gamif-toast {
      animation: none;
    }
  }

  .global-player-bar {
    position: fixed;
    bottom: 0;
    left: var(--sidebar-width, 0px);
    right: 0;
    z-index: 80;
  }
  :global(body:has(.global-player-bar)) {
    padding-bottom: 80px;
  }
</style>
