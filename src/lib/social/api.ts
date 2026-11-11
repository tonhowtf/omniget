import { invoke } from "@tauri-apps/api/core";

export type Platform = "x" | "instagram" | "tiktok" | "reddit" | "pinterest" | "github";
export type Extra = Record<string, unknown>;

export type Author = { id: string; handle: string; name: string; avatar?: string; verified: boolean };
export type Media = { kind: "image" | "video"; url: string; thumb?: string; width?: number; height?: number };
export type Stats = { likes?: number; replies?: number; reposts?: number; views?: number; saves?: number };

export type Post = {
  id: string;
  url: string;
  author: Author;
  text: string;
  created_at?: number;
  media: Media[];
  stats: Stats;
  liked?: boolean;
  reposted?: boolean;
  reply_to?: string;
  reposted_by?: Author;
  quoted?: Post;
  extra?: Extra;
};

export type Profile = {
  id: string;
  handle: string;
  name: string;
  avatar?: string;
  bio: string;
  url: string;
  verified: boolean;
  private: boolean;
  followers?: number;
  following?: number;
  posts?: number;
  likes?: number;
  followed_by_me?: boolean;
  follows_me?: boolean;
  extra?: Extra;
};

export type Comment = { id: string; author: Author; text: string; created_at?: number; likes?: number; replies?: number; extra?: Extra };

export type Page<T> = { count: number; items: T[]; next_cursor: string | null };
export type PostDetail = { post: Post | null; replies?: Page<Post>; comments?: Page<Comment> };
export type PlatformStatus = { platform: Platform; browser: boolean; logged_in: boolean | null; auth?: "token" | "username" | "none" };
export type GithubAuth = { has_token: boolean; username: string | null };

export type ApprovalRequest = {
  id: string;
  tool: string;
  platform: Platform;
  summary: string;
  details: Record<string, unknown> | null;
  client: string | null;
  created: number;
  expires_at: number;
};

export const PREFIX: Record<Platform, string> = { x: "x_", instagram: "ig_", tiktok: "tt_", reddit: "rd_", pinterest: "pin_", github: "gh_" };

export function tool(p: Platform, name: string): string {
  return `${PREFIX[p]}${name}`;
}

export const social = {
  status: () => invoke<{ platforms: PlatformStatus[] }>("social_status"),
  login: (platform: Platform) => invoke("social_login", { platform }),
  call: <T>(name: string, args: Record<string, unknown> = {}) => invoke<T>("social_call", { tool: name, args }),
  githubState: () => invoke<GithubAuth>("social_github_state"),
  githubConnect: (auth: { token?: string; username?: string }) => invoke<GithubAuth>("social_github_connect", auth),
  approvalList: () => invoke<ApprovalRequest[]>("social_approval_list"),
  approvalAnswer: (id: string, approve: boolean) => invoke("social_approval_answer", { id, approve }),
};

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function errKey(e: unknown): string {
  const code = errText(e).split(":")[0].trim();
  if (code.startsWith("UPSTREAM_")) return code === "UPSTREAM_CHANGED" ? "upstream_changed" : "upstream";
  const known = ["NOT_LOGGED_IN", "RATE_LIMITED", "NETWORK", "BROWSER_UNAVAILABLE", "REJECTED", "NOT_FOUND", "INVALID_ARGUMENTS"];
  return known.includes(code) ? code.toLowerCase() : "unknown";
}

export function errDetail(e: unknown): string {
  const t = errText(e);
  const i = t.indexOf(":");
  const rest = i > 0 && /^[A-Z_0-9]+$/.test(t.slice(0, i)) ? t.slice(i + 1).trim() : t;
  if (rest.startsWith("{")) {
    try {
      const message = (JSON.parse(rest) as { message?: unknown }).message;
      if (typeof message === "string" && message) return message;
    } catch {
      return rest;
    }
  }
  return rest;
}

export function compact(n: number | undefined, locale?: string): string {
  if (n == null) return "";
  try {
    return new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 }).format(n);
  } catch {
    return String(n);
  }
}

export function ago(ts: number | undefined, locale?: string): string {
  if (!ts) return "";
  const diff = Math.round(ts - Date.now() / 1000);
  const abs = Math.abs(diff);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31536000],
    ["month", 2592000],
    ["week", 604800],
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ];
  try {
    const f = new Intl.RelativeTimeFormat(locale, { numeric: "auto", style: "short" });
    for (const [u, s] of units) if (abs >= s) return f.format(Math.round(diff / s), u);
    return f.format(diff, "second");
  } catch {
    return new Date(ts * 1000).toLocaleString();
  }
}
