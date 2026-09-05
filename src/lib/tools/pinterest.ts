/**
 * Tipos e utilidades da categoria Pinterest (estudo 67). Espelham os
 * structs de `omniget-core/src/core/tools/pinterest/` e dos comandos
 * `tool_pin_*`; tudo que a UI repete (filtros, opções, rótulos) mora aqui.
 */

export type Media = { url: string; width: number; height: number };
export type Video = { hls: string | null; mp4: string | null; width: number; height: number; duration_ms: number; thumbnail: string | null };
export type Person = { id: string | null; username: string | null; name: string | null; avatar: string | null };
export type BoardRef = { id: string | null; name: string | null; url: string | null };
export type Rich = { site_name: string | null; title: string | null; url: string | null; description: string | null };
export type Attribution = { author_name: string | null; author_url: string | null; provider_name: string | null; title: string | null; url: string | null };
export type Extra = { index: number; kind: "image" | "video"; image: Media | null; video: Video | null };
export type AiInfo = { labeled: boolean; topics: string[]; keyword_level: number; keyword: string | null };

export type Pin = {
  id: string;
  url: string;
  title: string;
  description: string;
  alt_text: string;
  link: string | null;
  domain: string | null;
  created_at: string | null;
  kind: "image" | "gif" | "video" | "carousel" | "story";
  image_signature: string | null;
  image: Media | null;
  image_large: Media | null;
  thumb: string | null;
  video: Video | null;
  extras: Extra[];
  pinner: Person | null;
  creator: Person | null;
  board: BoardRef | null;
  saves: number;
  repins: number;
  comments: number;
  reactions: number;
  is_promoted: boolean;
  is_repin: boolean;
  ai: AiInfo;
  dominant_color: string | null;
  rich: Rich | null;
  attribution: Attribution | null;
  section: string | null;
};

export type Board = {
  id: string; name: string; url: string; description: string; pin_count: number; section_count: number; follower_count: number;
  privacy: string; cover: string | null; owner: Person | null; is_collaborative: boolean;
};
export type Section = { id: string; slug: string; title: string; pin_count: number };
export type User = {
  id: string; username: string; name: string; about: string; website: string | null; avatar: string | null; pin_count: number;
  board_count: number; follower_count: number; following_count: number; is_verified_merchant: boolean; is_private: boolean;
};

export type Target =
  | { kind: "pin"; id: string }
  | { kind: "board"; user: string; slug: string }
  | { kind: "section"; user: string; slug: string; section: string }
  | { kind: "user"; username: string }
  | { kind: "user_created"; username: string }
  | { kind: "search"; query: string; scope: string }
  | { kind: "short"; code: string };

export type Inspect = {
  target: Target; resolved_url: string; pin: Pin | null; board: Board | null; section: Section | null; sections: Section[];
  user: User | null; boards: Board[]; has_session: boolean;
};

export type Filters = { skip_promoted: boolean; ai_level: number; only_kind: "" | "image" | "video" | "gif"; min_width: number };
export const defaultFilters = (): Filters => ({ skip_promoted: true, ai_level: 2, only_kind: "", min_width: 0 });

export type DownloadOptions = {
  dest: string; images: boolean; videos: boolean; convert_webp: boolean; naming: "id" | "title" | "title-id"; sidecar: boolean;
  skip_downloaded: boolean; section_folders: boolean;
};
export const defaultDownload = (dest = ""): DownloadOptions => ({
  dest, images: true, videos: true, convert_webp: false, naming: "title-id", sidecar: false, skip_downloaded: true, section_folders: true,
});

export type ListOut = { title: string; target: Target; pins: Pin[]; guides: string[]; hidden: number };
export type PinFiles = { id: string; files: string[]; skipped: boolean; error: string | null };
export type ManyOut = { dest: string; downloaded: number; skipped: number; failed: PinFiles[]; files: number };
export type BackupOut = ManyOut & {
  title: string; total: number; hidden: number; csv_path: string | null; json_path: string | null; html_path: string | null; boards: number;
};
export type DupesOut = { title: string; scanned: number; groups: { kind: "exact" | "near"; distance: number; pins: Pin[] }[]; has_session: boolean };
export type Swatch = { hex: string; rgb: [number, number, number]; share: number };
export type PaletteOut = { title: string; pins_used: number; swatches: Swatch[]; dominant: Swatch[] };
export type ExportOut = { title: string; path: string; pins: number };
export type KeywordsOut = { term: string; suggestions: string[]; guides: string[]; common: [string, number][]; sample: number };
export type LinkCheck = { url: string; status: number | null; final_url: string | null; ok: boolean; error: string | null };
export type SourceOut = { pin: Pin; link: LinkCheck | null; wayback: string | null; reverse: [string, string][]; resolved_url: string };

/** Melhor imagem para mostrar num card. */
export function preview(p: Pin): string {
  return p.thumb ?? p.image_large?.url ?? p.image?.url ?? p.video?.thumbnail ?? "";
}

/** Chave i18n do tipo do pin. */
export function kindKey(p: Pin): string {
  return `tools.pinterest.kind_${p.kind}`;
}

/** Sinal de IA que vale a pena mostrar (rótulo oficial ou ferramenta citada). */
export function looksAi(p: Pin): boolean {
  return p.ai.labeled || p.ai.keyword_level >= 2;
}

export function fmtCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

export function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Luminância relativa para escolher texto claro/escuro sobre a cor. */
export function darkText(hex: string): boolean {
  const v = parseInt(hex.replace("#", ""), 16);
  const r = (v >> 16) & 255, g = (v >> 8) & 255, b = v & 255;
  return 0.299 * r + 0.587 * g + 0.114 * b > 150;
}

export function paletteCss(sw: Swatch[]): string {
  return `:root {\n${sw.map((s, i) => `  --color-${i + 1}: ${s.hex};`).join("\n")}\n}`;
}

const COOKIES_KEY = "omniget.pinterest.cookies";
export function loadCookies(): string {
  try {
    return localStorage.getItem(COOKIES_KEY) ?? "";
  } catch {
    return "";
  }
}
export function saveCookies(v: string): void {
  try {
    if (v.trim()) localStorage.setItem(COOKIES_KEY, v.trim());
    else localStorage.removeItem(COOKIES_KEY);
  } catch {
    /* storage indisponível */
  }
}
