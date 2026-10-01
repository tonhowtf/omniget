export type SmartPreset = {
  id: string;
  name: string;
  mode: "video" | "audio";
  height: number | null;
  subs: string;
  folder: string;
};

export type SmartState = { presets: SmartPreset[]; activeId: string };

const KEY = "omniget.download-tools.smart-mode.v1";

export const HEIGHTS: (number | null)[] = [null, 2160, 1440, 1080, 720, 480, 360];

function defaultState(): SmartState {
  return {
    activeId: "video",
    presets: [
      { id: "video", name: "", mode: "video", height: 1080, subs: "", folder: "" },
      { id: "video-subs", name: "", mode: "video", height: 1080, subs: "en", folder: "" },
      { id: "audio", name: "", mode: "audio", height: null, subs: "", folder: "" },
    ],
  };
}

export function validSubs(value: string): boolean {
  const v = value.trim();
  return v === "" || (/^[A-Za-z0-9][A-Za-z0-9_.,-]{0,63}$/.test(v) && !v.includes(",,"));
}

function sane(p: unknown): p is SmartPreset {
  const x = p as SmartPreset;
  return (
    !!x &&
    typeof x.id === "string" &&
    typeof x.name === "string" &&
    (x.mode === "video" || x.mode === "audio") &&
    (x.height === null || typeof x.height === "number") &&
    typeof x.subs === "string" &&
    typeof x.folder === "string"
  );
}

export function loadState(): SmartState {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return defaultState();
    const parsed = JSON.parse(raw) as SmartState;
    const presets = Array.isArray(parsed.presets) ? parsed.presets.filter(sane) : [];
    if (presets.length === 0) return defaultState();
    const activeId = presets.some((p) => p.id === parsed.activeId) ? parsed.activeId : presets[0].id;
    return { presets, activeId };
  } catch {
    return defaultState();
  }
}

export function saveState(state: SmartState): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
  }
}

export function downloadArgs(preset: SmartPreset, url: string, fallbackFolder: string) {
  const audio = preset.mode === "audio";
  return {
    url,
    outputDir: preset.folder || fallbackFolder,
    downloadMode: audio ? "audio" : null,
    quality: audio || preset.height === null ? "best" : `${preset.height}p`,
    formatId: null,
    referer: null,
    cookieSlug: null,
    subtitleLang: !audio && preset.subs.trim() ? preset.subs.trim() : null,
  };
}

export function linksIn(text: string): string[] {
  const found = text.match(/\b(?:https?:\/\/|magnet:\?)[^\s<>"']+/gi) ?? [];
  return [...new Set(found.map((u) => u.replace(/[),.;]+$/, "")))];
}
