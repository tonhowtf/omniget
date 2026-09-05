/**
 * Tipos e utilidades da categoria Instagram (espelham
 * `omniget-core/src/core/tools/instagram/`). A conta escolhida fica em
 * localStorage para todas as tools da categoria usarem a mesma sessão.
 */
import { invoke } from "@tauri-apps/api/core";
import { saveAs } from "$lib/tools/rt";

export type IgAccount = { slug: string; alias: string; captured_at_ms: number; cookie_count: number; has_session: boolean; user_id: string };

export type MediaFile = { url: string; kind: "image" | "video"; width: number; height: number; poster: string | null; pk: string };

export type MediaItem = {
  pk: string;
  code: string;
  media_type: number;
  product_type: string;
  taken_at: number;
  expiring_at: number | null;
  caption: string;
  like_count: number;
  comment_count: number;
  play_count: number;
  owner_id: string;
  username: string;
  full_name: string;
  thumbnail: string;
  files: MediaFile[];
  duration: number;
  location: string | null;
  url: string;
  width: number;
  height: number;
  hashtags: string[];
  mentions: string[];
  is_paid_partnership: boolean;
  coauthors: string[];
  title: string | null;
};

export type DownloadOptions = { caption_txt: boolean; metadata_json: boolean; audio_only: "" | "m4a" | "mp3"; skip_existing: boolean; per_user_folder: boolean };
export type DownloadResult = { files: string[]; skipped: number; failed: string[]; dest: string };

export type UserInfo = {
  pk: string;
  username: string;
  full_name: string;
  biography: string;
  external_url: string;
  follower_count: number;
  following_count: number;
  media_count: number;
  total_clips: number;
  is_private: boolean;
  is_verified: boolean;
  is_business: boolean;
  category: string;
  profile_pic_url: string;
  profile_pic_hd: string;
  followed_by_viewer: boolean;
  follows_viewer: boolean;
  has_highlights: boolean;
  is_self: boolean;
};

export type MiniUser = { pk: string; username: string; full_name: string; is_private: boolean; is_verified: boolean; profile_pic_url: string };
export type Highlight = { id: string; title: string; cover: string; media_count: number; created_at: number };
export type Reel = { id: string; title: string | null; username: string; user_id: string; items: MediaItem[]; expiring_at: number | null; close_friends: boolean };
export type TrayEntry = { user_id: string; username: string; full_name: string; profile_pic_url: string; latest_reel_media: number; seen: number; close_friends: boolean };

export type FollowAnalysis = {
  followers_count: number;
  following_count: number;
  not_following_back: MiniUser[];
  fans: MiniUser[];
  mutuals: MiniUser[];
  whitelisted: number;
  followers: MiniUser[];
  following: MiniUser[];
};
export type Pacing = { delay_min_ms: number; delay_max_ms: number; pause_every: number; pause_ms: number; daily_cap: number };
export const DEFAULT_PACING: Pacing = { delay_min_ms: 6000, delay_max_ms: 14000, pause_every: 5, pause_ms: 300000, daily_cap: 100 };
export type ActionReport = { done: MiniUser[]; failed: [MiniUser, string][]; remaining: MiniUser[]; stopped: string; actions_today: number };
export type SnapshotMeta = { file: string; taken_at: number; followers: number; following: number };
export type SnapshotDiff = { from: number; to: number; new_followers: MiniUser[]; lost_followers: MiniUser[]; new_following: MiniUser[]; lost_following: MiniUser[] };
export type GhostReport = { posts_checked: number; followers_total: number; engaged: number; ghosts: MiniUser[]; top_fans: [MiniUser, number][] };

export type ExportUser = { username: string; href: string; timestamp: number };
export type ExportReport = {
  source: string;
  files_found: string[];
  followers: ExportUser[];
  following: ExportUser[];
  not_following_back: ExportUser[];
  fans: ExportUser[];
  mutuals: number;
  pending_sent: ExportUser[];
  close_friends: ExportUser[];
  blocked: ExportUser[];
  recently_unfollowed: ExportUser[];
  received_requests: ExportUser[];
  restricted: ExportUser[];
  hide_story_from: ExportUser[];
  removed_suggestions: ExportUser[];
  followers_by_month: [string, number][];
};

export type PostBrief = { code: string; url: string; thumbnail: string; taken_at: number; likes: number; comments: number; plays: number; kind: string; caption: string };
export type ProfileStats = {
  user: UserInfo;
  posts_analyzed: number;
  span_days: number;
  posts_per_week: number;
  avg_likes: number;
  avg_comments: number;
  avg_plays: number;
  median_likes: number;
  engagement_rate: number;
  comment_ratio: number;
  follow_ratio: number;
  share_photo: number;
  share_video: number;
  share_carousel: number;
  avg_caption_len: number;
  avg_hashtags: number;
  top_hashtags: [string, number][];
  top_mentions: [string, number][];
  weekday_counts: number[];
  hour_counts: number[];
  weekday_engagement: number[];
  best_weekday: number;
  best_hour: number;
  top_posts: PostBrief[];
  paid_partnerships: number;
  first_post_at: number;
  last_post_at: number;
};

export type Comment = { pk: string; text: string; created_at: number; user: MiniUser; like_count: number; reply_count: number; mentions: string[] };
export type GiveawayRules = { winners: number; unique_users: boolean; min_mentions: number; keyword: string; exclude: string[]; owner_username: string };
export type GiveawayResult = { eligible: number; winners: Comment[]; seed: number };
export type TagInfo = { name: string; media_count: number; formatted_media_count: string; profile_pic_url: string; following: boolean };

export type PublishRequest = { kind: "photo" | "video" | "reel" | "story" | "carousel"; files: string[]; caption: string; share_to_feed: boolean; disable_comments: boolean; hide_like_counts: boolean; alt_text: string };
export type PublishResult = { media_id: string; code: string; url: string };
export type GraphAuth = { access_token: string; ig_user_id: string };
export type ScheduledPost = {
  id: string;
  run_at: number;
  request: PublishRequest;
  mode: "web" | "graph";
  account_slug: string | null;
  graph: GraphAuth | null;
  status: string;
  result: PublishResult | null;
  error: string | null;
  created_at: number;
};

/** Conta escolhida para toda a categoria (slug do bucket instagram.com). */
function readSlug(): string | null {
  try {
    return localStorage.getItem("ig.account");
  } catch {
    return null;
  }
}

export const igState = $state<{ slug: string | null; me: UserInfo | null; accounts: IgAccount[] }>({ slug: readSlug(), me: null, accounts: [] });

export function setAccount(slug: string | null) {
  igState.slug = slug;
  igState.me = null;
  try {
    if (slug) localStorage.setItem("ig.account", slug);
    else localStorage.removeItem("ig.account");
  } catch {
    /* sem localStorage */
  }
}

export async function loadAccounts(): Promise<IgAccount[]> {
  const list = await invoke<IgAccount[]>("tool_ig_accounts");
  igState.accounts = list;
  if (!list.some((a) => a.slug === igState.slug)) {
    setAccount(list.find((a) => a.has_session)?.slug ?? list[0]?.slug ?? null);
  }
  return list;
}

export function slugArg(): string | null {
  return igState.slug;
}

let jobCounter = 0;
export function jobId(prefix: string): string {
  jobCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${jobCounter}`;
}

export async function cancelJob(job: string) {
  await invoke("tool_ig_cancel", { job });
}

export function remember(key: string, value: string) {
  try {
    localStorage.setItem(`ig.${key}`, value);
  } catch {
    /* ignore */
  }
}

export function recall(key: string, fallback = ""): string {
  try {
    return localStorage.getItem(`ig.${key}`) ?? fallback;
  } catch {
    return fallback;
  }
}

export function defaultDownloadOptions(): DownloadOptions {
  const saved = recall("dlopts");
  if (saved) {
    try {
      return { caption_txt: false, metadata_json: false, audio_only: "", skip_existing: true, per_user_folder: false, ...JSON.parse(saved) };
    } catch {
      /* fallthrough */
    }
  }
  return { caption_txt: false, metadata_json: false, audio_only: "", skip_existing: true, per_user_folder: false };
}

/** Salva uma lista como CSV escolhendo o arquivo. */
export async function exportCsv(name: string, header: string[], rows: string[][]): Promise<string | null> {
  const path = await saveAs(`${name}.csv`, [{ name: "CSV", extensions: ["csv"] }]);
  if (!path) return null;
  return invoke<string>("tool_ig_write_csv", { path, rows: [header, ...rows] });
}

export function usersCsv(users: MiniUser[]): string[][] {
  return users.map((u) => [u.username, u.full_name, u.pk, u.is_private ? "private" : "public", u.is_verified ? "verified" : "", `https://www.instagram.com/${u.username}/`]);
}

export function fmtDate(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString();
}

export function fmtDay(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleDateString();
}

export function n(v: number | undefined | null): string {
  return (v ?? 0).toLocaleString();
}

export function compact(v: number): string {
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (v >= 10_000) return `${Math.round(v / 1000)}K`;
  if (v >= 1000) return `${(v / 1000).toFixed(1)}K`;
  return String(v);
}

export function profileUrl(username: string): string {
  return `https://www.instagram.com/${username}/`;
}

export function kindLabel(item: MediaItem): string {
  if (item.product_type === "story") return "story";
  if (item.media_type === 8) return "carousel";
  if (item.product_type === "clips") return "reel";
  if (item.product_type === "igtv") return "igtv";
  if (item.media_type === 2) return "video";
  return "photo";
}
