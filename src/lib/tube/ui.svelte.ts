// In-app router for /youtube. Each face (video, music) keeps its own stack,
// so switching faces returns to where you were. Deep links use
// `/youtube?v=<id>`, `/youtube?list=<id>`, `/youtube?c=<id>`, `/youtube?q=<text>`,
// `/youtube?m=1` (music face) — see `routeFromUrl`.
import type { ChannelTabName, MusicLibraryKind, MusicSearchFilter, SearchFilter } from "./api";

export type Face = "video" | "music";

export type Route =
  | { name: "home" }
  | { name: "search"; q: string; filter?: SearchFilter }
  | { name: "watch"; id: string; list?: string | null; index?: number | null; t?: number | null }
  | { name: "channel"; id: string; tab?: ChannelTabName }
  | { name: "playlist"; id: string }
  | { name: "subscriptions" }
  | { name: "shorts" }
  | { name: "library" }
  | { name: "history" }
  | { name: "settings"; section?: string }
  | { name: "import" }
  | { name: "m_home" }
  | { name: "m_explore" }
  | { name: "m_library"; tab?: MusicLibraryKind }
  | { name: "m_search"; q: string; filter?: MusicSearchFilter }
  | { name: "m_album"; id: string }
  | { name: "m_artist"; id: string }
  | { name: "m_playlist"; id: string }
  | { name: "m_browse"; id: string; params?: string | null; title?: string | null };

export const ui = $state({
  face: "video" as Face,
  stacks: { video: [{ name: "home" }] as Route[], music: [{ name: "m_home" }] as Route[] },
  /** Guide drawer (240px) open; otherwise the 72px mini-guide shows. */
  guideOpen: true,
  /** Music "Now playing" sheet open. */
  nowPlaying: false,
  /** Bumped on every navigation, for scroll restore. */
  nav: 0,
});

export function current(): Route {
  const s = ui.stacks[ui.face];
  return s[s.length - 1];
}

function faceOf(r: Route): Face {
  return r.name.startsWith("m_") ? "music" : "video";
}

/** Navigate. Same-name top routes (home, subscriptions...) replace instead of piling up. */
export function go(r: Route, opts: { replace?: boolean } = {}) {
  const face = faceOf(r);
  if (r.name === "settings" || r.name === "import") {
    // settings live on the video stack but are reachable from both faces
  }
  ui.face = r.name === "settings" || r.name === "import" ? ui.face : face;
  const stack = ui.stacks[r.name === "settings" || r.name === "import" ? ui.face : face];
  const top = stack[stack.length - 1];
  if (opts.replace || (top && JSON.stringify(top) === JSON.stringify(r))) {
    stack[stack.length - 1] = r;
  } else {
    stack.push(r);
    if (stack.length > 50) stack.splice(1, stack.length - 50);
  }
  if (r.name !== "watch") ui.nowPlaying = false;
  ui.nav++;
}

/** Root destinations of the guide reset the stack. */
export function goRoot(r: Route) {
  const face = faceOf(r);
  ui.face = face;
  ui.stacks[face] = [r];
  ui.nowPlaying = false;
  ui.nav++;
}

export function back(): boolean {
  const s = ui.stacks[ui.face];
  if (s.length <= 1) return false;
  s.pop();
  ui.nav++;
  return true;
}

export function canGoBack(): boolean {
  return ui.stacks[ui.face].length > 1;
}

export function setFace(face: Face) {
  ui.face = face;
  ui.nowPlaying = false;
  ui.nav++;
}

/** Parse `/youtube?...` query params into a route (deep links from other screens). */
export function routeFromUrl(params: URLSearchParams): Route | null {
  const v = params.get("v");
  if (v) {
    const t = Number(params.get("t") ?? "");
    return { name: "watch", id: v, list: params.get("list"), t: Number.isFinite(t) && t > 0 ? t : null };
  }
  const list = params.get("list");
  if (list) return params.get("m") ? { name: "m_playlist", id: list } : { name: "playlist", id: list };
  const c = params.get("c");
  if (c) return { name: "channel", id: c };
  const q = params.get("q");
  if (q) return params.get("m") ? { name: "m_search", q } : { name: "search", q };
  if (params.get("m")) return { name: "m_home" };
  return null;
}

/** Turn any youtube.com / youtu.be / music.youtube.com URL into a route. */
export function routeFromYoutubeUrl(raw: string): Route | null {
  let u: URL;
  try {
    u = new URL(raw.trim());
  } catch {
    return null;
  }
  const host = u.hostname.replace(/^www\.|^m\./, "");
  const music = host === "music.youtube.com";
  if (host === "youtu.be") {
    const id = u.pathname.slice(1).split("/")[0];
    return id ? { name: "watch", id, t: Number(u.searchParams.get("t")) || null } : null;
  }
  if (!host.endsWith("youtube.com")) return null;
  const p = u.pathname;
  if (p === "/watch" && u.searchParams.get("v")) {
    return { name: "watch", id: u.searchParams.get("v")!, list: u.searchParams.get("list"), t: Number(u.searchParams.get("t")?.replace("s", "")) || null };
  }
  const shorts = p.match(/^\/shorts\/([\w-]{6,})/);
  if (shorts) return { name: "watch", id: shorts[1] };
  const live = p.match(/^\/live\/([\w-]{6,})/);
  if (live) return { name: "watch", id: live[1] };
  if (p === "/playlist" && u.searchParams.get("list")) {
    return music ? { name: "m_playlist", id: u.searchParams.get("list")! } : { name: "playlist", id: u.searchParams.get("list")! };
  }
  if (music && p.startsWith("/browse/")) {
    const id = p.slice(8);
    return id.startsWith("MPRE") ? { name: "m_album", id } : { name: "m_artist", id };
  }
  if (music && p.startsWith("/channel/")) return { name: "m_artist", id: p.slice(9) };
  const ch = p.match(/^\/(channel\/[\w-]+|@[^/]+|c\/[^/]+|user\/[^/]+)/);
  if (ch) return { name: "channel", id: ch[1].startsWith("channel/") ? ch[1].slice(8) : ch[1] };
  if (p === "/results" && u.searchParams.get("search_query")) return { name: "search", q: u.searchParams.get("search_query")! };
  if (music && p === "/search" && u.searchParams.get("q")) return { name: "m_search", q: u.searchParams.get("q")! };
  return null;
}
