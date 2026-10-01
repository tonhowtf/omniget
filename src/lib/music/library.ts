import type { Album, Artist, LrcLine, Track } from "./types";

function albumArtistOf(t: Track): string {
  return (t.album_artist || t.artist || "").trim();
}

export function albumKey(t: Track): string {
  const who = t.album_artist.trim() ? t.album_artist.trim().toLowerCase() : `dir:${t.folder}`;
  return `${who}\u0000${(t.album || "").trim().toLowerCase()}`;
}

function byDiscTrack(a: Track, b: Track): number {
  return (
    (a.disc_no ?? 1) - (b.disc_no ?? 1) ||
    (a.track_no ?? 9999) - (b.track_no ?? 9999) ||
    a.title.localeCompare(b.title)
  );
}

export function groupAlbums(tracks: Track[]): Album[] {
  const map = new Map<string, Album>();
  for (const t of tracks) {
    if (!t.album.trim()) continue;
    const key = albumKey(t);
    let a = map.get(key);
    if (!a) {
      a = {
        key,
        title: t.album.trim(),
        artist: albumArtistOf(t),
        year: t.year,
        cover: t.cover,
        tracks: [],
        duration_ms: 0,
        added_at: t.added_at,
        codec: t.codec,
      };
      map.set(key, a);
    }
    a.tracks.push(t);
    a.duration_ms += t.duration_ms;
    if (!a.cover && t.cover) a.cover = t.cover;
    if (!a.year && t.year) a.year = t.year;
    if (t.added_at > a.added_at) a.added_at = t.added_at;
  }
  for (const a of map.values()) {
    a.tracks.sort(byDiscTrack);
    const artists = new Set(a.tracks.map((t) => t.artist));
    if (artists.size > 3 && !a.tracks.some((t) => t.album_artist)) a.artist = "Various Artists";
    if (a.tracks.some((t) => t.codec !== a.codec)) a.codec = "";
  }
  return [...map.values()];
}

export function groupArtists(tracks: Track[], albums: Album[]): Artist[] {
  const map = new Map<string, Artist>();
  for (const t of tracks) {
    const names = splitArtists(t.artist || t.album_artist);
    for (const name of names) {
      const k = name.toLowerCase();
      let a = map.get(k);
      if (!a) {
        a = { name, tracks: [], albums: [], cover: null, plays: 0 };
        map.set(k, a);
      }
      a.tracks.push(t);
      a.plays += t.play_count;
      if (!a.cover && t.cover) a.cover = t.cover;
    }
  }
  for (const al of albums) {
    const a = map.get(al.artist.toLowerCase());
    if (a) a.albums.push(al);
  }
  for (const a of map.values()) a.albums.sort((x, y) => (y.year ?? 0) - (x.year ?? 0));
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

export function splitArtists(s: string): string[] {
  const clean = s.trim();
  if (!clean) return ["Unknown Artist"];
  const parts = clean
    .split(/\s+(?:feat\.?|ft\.?|featuring|with|x|vs\.?)\s+|;\s*|\s+\/\s+/i)
    .map((p) => p.trim())
    .filter(Boolean);
  return parts.length ? parts : [clean];
}

export function groupBy<T>(items: T[], key: (t: T) => string): Map<string, T[]> {
  const m = new Map<string, T[]>();
  for (const it of items) {
    const k = key(it);
    const arr = m.get(k);
    if (arr) arr.push(it);
    else m.set(k, [it]);
  }
  return m;
}

export function genres(tracks: Track[]): { name: string; tracks: Track[] }[] {
  const m = new Map<string, Track[]>();
  for (const t of tracks) {
    for (const g of (t.genre || "").split(/[;,/]/).map((x) => x.trim()).filter(Boolean)) {
      const k = g.toLowerCase();
      const arr = m.get(k);
      if (arr) arr.push(t);
      else m.set(k, [t]);
    }
  }
  return [...m.entries()]
    .map(([, ts]) => ({ name: ts[0].genre.split(/[;,/]/)[0].trim() || ts[0].genre, tracks: ts }))
    .sort((a, b) => b.tracks.length - a.tracks.length);
}

export function fold(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

export function searchTracks(tracks: Track[], query: string): Track[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return tracks;
  return tracks.filter((t) => {
    const hay = fold(`${t.title} ${t.artist} ${t.album} ${t.album_artist} ${t.genre} ${t.composer} ${t.path.split(/[\\/]/).pop() ?? ""}`);
    return words.every((w) => hay.includes(w));
  });
}

export type SortKey =
  | "title" | "artist" | "album" | "year" | "duration" | "added" | "plays" | "rating" | "bpm" | "key" | "last_played" | "track";

export function sortTracks(tracks: Track[], key: SortKey, desc = false): Track[] {
  const dir = desc ? -1 : 1;
  const str = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
  const cmp: Record<SortKey, (a: Track, b: Track) => number> = {
    title: (a, b) => str(a.title, b.title),
    artist: (a, b) => str(a.artist, b.artist) || str(a.album, b.album) || byDiscTrack(a, b),
    album: (a, b) => str(a.album, b.album) || byDiscTrack(a, b),
    track: byDiscTrack,
    year: (a, b) => (a.year ?? 0) - (b.year ?? 0),
    duration: (a, b) => a.duration_ms - b.duration_ms,
    added: (a, b) => a.added_at - b.added_at,
    plays: (a, b) => a.play_count - b.play_count,
    rating: (a, b) => a.rating - b.rating,
    bpm: (a, b) => (a.bpm ?? 0) - (b.bpm ?? 0),
    key: (a, b) => str(a.camelot, b.camelot),
    last_played: (a, b) => (a.last_played ?? 0) - (b.last_played ?? 0),
  };
  const f = cmp[key];
  return [...tracks].sort((a, b) => f(a, b) * dir);
}

export function fmtTime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) ms = 0;
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
}

export function fmtLong(ms: number): string {
  const min = Math.round(ms / 60000);
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  if (h < 48) return `${h} h ${min % 60} min`;
  return `${Math.round(h / 24)} d`;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${u[i]}`;
}

export function qualityLabel(t: Pick<Track, "codec" | "bitrate" | "sample_rate" | "bit_depth">): string {
  const lossless = ["FLAC", "ALAC", "WAV", "AIFF", "WavPack", "APE"].includes(t.codec);
  if (lossless) {
    const bits = t.bit_depth ? `${t.bit_depth}-bit` : "";
    const khz = t.sample_rate ? `${(t.sample_rate / 1000).toFixed(t.sample_rate % 1000 ? 1 : 0)} kHz` : "";
    return [t.codec, bits, khz].filter(Boolean).join(" · ");
  }
  return [t.codec, t.bitrate ? `${t.bitrate} kbps` : ""].filter(Boolean).join(" · ");
}

export function looksUpscaled(t: Pick<Track, "codec" | "cutoff_hz" | "sample_rate">): boolean {
  const lossless = ["FLAC", "ALAC", "WAV", "AIFF", "WavPack", "APE"].includes(t.codec);
  if (!lossless || !t.cutoff_hz) return false;
  const nyq = (t.sample_rate ?? 44100) / 2;
  return t.cutoff_hz < Math.min(19500, nyq - 1500);
}

export function lineAt(lines: LrcLine[], ms: number): number {
  if (!lines.length || ms < lines[0].time_ms) return -1;
  let lo = 0;
  let hi = lines.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (lines[mid].time_ms <= ms) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

export function shuffled<T>(items: T[], first?: T): T[] {
  const a = items.filter((x) => x !== first);
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return first !== undefined ? [first, ...a] : a;
}

export function camelotCompatible(a: string, b: string): boolean {
  const pa = /^(\d{1,2})([AB])$/.exec(a);
  const pb = /^(\d{1,2})([AB])$/.exec(b);
  if (!pa || !pb) return false;
  const na = Number(pa[1]);
  const nb = Number(pb[1]);
  if (pa[2] === pb[2]) {
    const d = Math.abs(na - nb);
    return d === 0 || d === 1 || d === 11;
  }
  return na === nb;
}
