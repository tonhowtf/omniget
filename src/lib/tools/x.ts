/**
 * Tipos e utilidades da categoria X / Twitter (estudo 67). Espelham os
 * structs de `omniget-core/src/core/tools/x/`.
 */
import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { errText } from "$lib/tools/rt";

export type XUser = {
  id: string;
  handle: string;
  name: string;
  avatar: string;
  banner: string;
  bio: string;
  location: string;
  website: string;
  joined: string;
  followers: number;
  following: number;
  posts: number;
  likes: number;
  media_count: number;
  verified: boolean;
  protected: boolean;
  followed_by_me: boolean | null;
  follows_me: boolean | null;
};

export type XMedia = { kind: "photo" | "video" | "gif"; url: string; thumb: string; width: number; height: number; duration_ms: number; alt: string };

export type XPost = {
  id: string;
  url: string;
  text: string;
  created_at: string;
  timestamp: number;
  author: XUser;
  likes: number;
  reposts: number;
  replies: number;
  quotes: number;
  views: number;
  bookmarks: number;
  lang: string;
  media: XMedia[];
  quote: XPost | null;
  reply_to_id: string | null;
  reply_to_handle: string | null;
  conversation_id: string | null;
  reposted_by: string | null;
  source: string;
  hashtags: string[];
  mentions: string[];
  links: string[];
};

export type XSession = { logged_in: boolean; user_id: string | null; user: XUser | null; query_ids_cached: number; query_ids_age_secs: number | null };

export type ExportFormat = "md" | "html" | "json" | "txt" | "csv";

/** Traduz os erros com prefixo do backend (`X_LOGIN_REQUIRED`, `X_RATE_LIMIT:n`). */
export function xErr(e: unknown): string {
  const raw = errText(e);
  const tr = get(t) as (k: string, v?: Record<string, unknown>) => string;
  if (raw.includes("X_LOGIN_REQUIRED")) return tr("tools.x.err_login");
  const m = raw.match(/X_RATE_LIMIT:(\d+)/);
  if (m) return tr("tools.x.err_rate", { mins: Math.max(1, Math.ceil(Number(m[1]) / 60)) });
  if (raw.includes("GROK_NO_KEY")) return tr("tools.x.grok_no_key");
  return raw;
}

export function fmtN(n: number | null | undefined): string {
  const v = n ?? 0;
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(v >= 1e5 ? 0 : 1)}k`;
  return String(v);
}

export function fmtDate(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { day: "2-digit", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function extOf(format: ExportFormat): string {
  return format === "txt" ? "txt" : format;
}

export async function session(): Promise<XSession> {
  return invoke<XSession>("tool_x_session");
}

type AuthCookie = { name: string; value: string; domain: string; path: string; httpOnly?: boolean; secure?: boolean };

/**
 * Abre a janela de login do X e guarda os cookies no bucket `x.com`, o mesmo
 * que o download de posts já usa.
 */
export async function loginToX(): Promise<boolean> {
  const result = await invoke<{ cookies: AuthCookie[]; finalUrl: string }>("open_auth_webview", {
    request: {
      url: "https://x.com/i/flow/login",
      title: "X",
      cookieDomains: ["x.com", "twitter.com"],
      successUrlContains: null,
      waitForCookie: "auth_token",
      initializationScript: null,
      width: 720,
      height: 760,
    },
  });
  const cookies = result.cookies.filter((c) => c.name && c.value);
  if (!cookies.some((c) => c.name === "auth_token")) return false;
  const payload = cookies.map((c) => ({
    domain: c.domain || ".x.com",
    name: c.name,
    value: c.value,
    path: c.path || "/",
    secure: c.secure ?? true,
    httpOnly: c.httpOnly ?? false,
    hostOnly: false,
    session: false,
    expirationDate: Math.floor(Date.now() / 1000) + 3600 * 24 * 365,
  }));
  await invoke("cookies_import", { request: { content: JSON.stringify(payload), source_url: "https://x.com", source_label: "OmniGet login", alias: null } });
  return true;
}

export function postIdFrom(input: string): string | null {
  const s = input.trim();
  if (/^\d+$/.test(s)) return s;
  const m = s.match(/(?:status(?:es)?|i\/web\/status)\/(\d+)/);
  return m ? m[1] : null;
}

export function handleFrom(input: string): string | null {
  const s = input.trim().replace(/^@/, "");
  if (/^[A-Za-z0-9_]{1,15}$/.test(s)) return s;
  const m = s.match(/(?:x\.com|twitter\.com)\/(?:#!\/)?@?([A-Za-z0-9_]{1,15})(?:[/?#]|$)/);
  return m ? m[1] : null;
}
