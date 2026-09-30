// YouTube's keyboard shortcuts for /youtube (spec §5). Installed by TubeApp.
// They drive the video when the watch page (or the mini player, if it was the
// last thing playing) is up, the music player otherwise.
import { back, current, ui } from "./ui.svelte";
import * as M from "./music.svelte";
import * as V from "./video.svelte";

export const keys = $state({
  /** Shortcut sheet open ("?"). */
  help: false,
});

let focusSearch: (() => void) | null = null;

/** The masthead registers how to focus its search box ("/"). */
export function onFocusSearch(fn: () => void): () => void {
  focusSearch = fn;
  return () => {
    if (focusSearch === fn) focusSearch = null;
  };
}

function typing(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (!t) return false;
  const tag = t.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || t.isContentEditable;
}

/** A focused control that already reacts to Space/Enter. */
function onControl(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === "BUTTON" || t.tagName === "A" || t.getAttribute("role") === "slider" || t.getAttribute("role") === "tab");
}

export function keyTarget(): "video" | "music" | null {
  const onWatch = current().name === "watch";
  if (onWatch && V.vplayer.videoId) return "video";
  if (V.vplayer.videoId && V.vplayer.mini && V.vplayer.active) return "video";
  if (M.currentSong()) return "music";
  if (V.vplayer.videoId) return "video";
  return null;
}

export function handleKey(e: KeyboardEvent): boolean {
  if (e.defaultPrevented || typing(e)) return false;
  if (e.ctrlKey || e.metaKey) return false;
  if (e.altKey) {
    if (e.key === "ArrowLeft") return back();
    return false;
  }
  const k = e.key;
  if (k === "/") {
    focusSearch?.();
    return !!focusSearch;
  }
  if (k === "?") {
    keys.help = !keys.help;
    return true;
  }
  if (k === "Escape") {
    if (keys.help) {
      keys.help = false;
      return true;
    }
    if (V.vplayer.fullscreen) {
      void V.setFullscreen(false);
      return true;
    }
    if (ui.nowPlaying) {
      ui.nowPlaying = false;
      return true;
    }
    return false;
  }
  const tgt = keyTarget();
  if (!tgt) return false;
  if (e.shiftKey && (k === "N" || k === "n")) {
    if (tgt === "video") V.next();
    else M.next();
    return true;
  }
  if (e.shiftKey && (k === "P" || k === "p")) {
    if (tgt === "video") V.prev();
    else M.prev();
    return true;
  }
  if (tgt === "music") return musicKey(e, k);
  return videoKey(e, k);
}

function videoKey(e: KeyboardEvent, k: string): boolean {
  switch (k) {
    case " ":
      if (onControl(e)) return false;
      V.toggle();
      return true;
    case "k":
    case "K":
      V.toggle();
      return true;
    case "j":
    case "J":
      V.seekBy(-10);
      return true;
    case "l":
    case "L":
      V.seekBy(10);
      return true;
    case "ArrowLeft":
      if (onControl(e)) return false;
      V.seekBy(-5);
      return true;
    case "ArrowRight":
      if (onControl(e)) return false;
      V.seekBy(5);
      return true;
    case "ArrowUp":
      if (current().name !== "watch" && !V.vplayer.fullscreen) return false;
      V.nudgeVolume(0.05);
      return true;
    case "ArrowDown":
      if (current().name !== "watch" && !V.vplayer.fullscreen) return false;
      V.nudgeVolume(-0.05);
      return true;
    case "m":
    case "M":
      V.toggleMute();
      return true;
    case "f":
    case "F":
      V.toggleFullscreen();
      return true;
    case "t":
    case "T":
      if (current().name !== "watch") return false;
      V.toggleTheater();
      return true;
    case "i":
    case "I":
      if (current().name === "watch") {
        // leaving the watch page is what turns on the mini player
        if (!back()) return false;
      } else if (V.vplayer.mini) V.expandMini();
      return true;
    case "c":
    case "C":
      V.toggleCaptions();
      return true;
    case "<":
    case ",":
      if (k === "," && !e.shiftKey) return false;
      V.stepRate(-1);
      return true;
    case ">":
    case ".":
      if (k === "." && !e.shiftKey) return false;
      V.stepRate(1);
      return true;
    case "Enter":
      if (onControl(e)) return false;
      return V.sbEnter();
    case "Home":
      V.seek(0);
      return true;
    case "End":
      V.seek(V.vplayer.duration - 1);
      return true;
  }
  if (/^[0-9]$/.test(k)) {
    V.seekFrac(Number(k) / 10);
    return true;
  }
  return false;
}

function musicKey(e: KeyboardEvent, k: string): boolean {
  switch (k) {
    case " ":
      if (onControl(e)) return false;
      M.toggle();
      return true;
    case "k":
    case "K":
      M.toggle();
      return true;
    case "j":
    case "J":
      M.seekBy(-10);
      return true;
    case "l":
    case "L":
      M.seekBy(10);
      return true;
    case "ArrowLeft":
      if (onControl(e)) return false;
      M.seekBy(-5);
      return true;
    case "ArrowRight":
      if (onControl(e)) return false;
      M.seekBy(5);
      return true;
    case "m":
    case "M":
      M.toggleMute();
      return true;
    case "s":
    case "S":
      M.toggleShuffle();
      return true;
    case "r":
    case "R":
      M.cycleRepeat();
      return true;
    case "+":
    case "=": {
      const s = M.currentSong();
      if (s) void M.rateSong(s, s.liked === true ? null : true).catch(() => {});
      return !!s;
    }
    case "-":
    case "_": {
      const s = M.currentSong();
      if (s) void M.rateSong(s, s.liked === false ? null : false).catch(() => {});
      return !!s;
    }
  }
  if (/^[0-9]$/.test(k) && M.mplayer.duration > 0) {
    M.seek((M.mplayer.duration * Number(k)) / 10);
    return true;
  }
  return false;
}

/** Global listener while /youtube is mounted. */
export function installKeys(): () => void {
  const h = (e: KeyboardEvent) => {
    if (handleKey(e)) e.preventDefault();
  };
  const mouse = (e: MouseEvent) => {
    // mouse "back" button
    if (e.button === 3 && back()) e.preventDefault();
  };
  window.addEventListener("keydown", h);
  window.addEventListener("mouseup", mouse);
  return () => {
    window.removeEventListener("keydown", h);
    window.removeEventListener("mouseup", mouse);
  };
}
