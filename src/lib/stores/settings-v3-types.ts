export type Revision = string;
type RecoveryId = string;

export type SettingsSection =
  | "general" | "appearance" | "downloads"
  | "ai" | "integrations" | "advanced";

type SecretOperation =
  | { op: "keep" }
  | { op: "set"; value: string }
  | { op: "remove" };

export type SettingsSecretOperations = {
  proxyPassword?: SecretOperation;
  twitterManualCookie?: SecretOperation;
};

type FieldErrorCode =
  | "unknown_field" | "protected_field" | "invalid_type"
  | "out_of_range" | "invalid_enum" | "invalid_value"
  | "conflict" | "required";
type FieldError = { path: string; code: FieldErrorCode };

export type LoadStatus =
  | { kind: "absent" }
  | { kind: "loaded" }
  | { kind: "recovered"; fields: string[]; recoveryId: RecoveryId }
  | {
      kind: "failed";
      code: "read_failed" | "invalid_document" | "invalid_revision" | "backup_failed";
      recoveryId: RecoveryId | null;
    };

export type EffectId =
  | "network.proxy" | "downloads.segments" | "downloads.queue"
  | "shortcuts.registration" | "app.autostart" | "integrations.rpc"
  | "llm.watchdog" | "llm.jobs" | "bridge.lifecycle";
type EffectState = "queued" | "applying" | "failed" | "restart_required";
export type PendingEffect = {
  id: EffectId;
  targetRevision: Revision;
  state: EffectState;
  code: "apply_failed" | "not_available" | "restart_required" | null;
  retryable: boolean;
};

export type ReadySnapshot = {
  settings: SettingsPublicV3;
  revision: Revision;
  loadStatus: Exclude<LoadStatus, { kind: "failed" }>;
  applied: boolean;
  pendingEffects: PendingEffect[];
  fieldErrors: [];
};
type FailedSnapshot = {
  settings: null;
  revision: Revision | null;
  loadStatus: Extract<LoadStatus, { kind: "failed" }>;
  applied: false;
  pendingEffects: [];
  fieldErrors: [];
};
export type SettingsSnapshot = ReadySnapshot | FailedSnapshot;

export type ApiError =
  | { kind: "validation"; code: "invalid_request"; fieldErrors: FieldError[] }
  | { kind: "conflict"; code: "revision_conflict"; currentRevision: Revision; fieldErrors: [] }
  | {
      kind: "unavailable";
      code: "profile_read_only" | "load_failed" | "recovery_required" | "backup_failed" | "revision_overflow";
      fieldErrors: [];
    }
  | {
      kind: "persistence";
      code: "create_failed" | "write_failed" | "sync_failed" | "replace_failed";
      committed: false;
      fieldErrors: [];
    }
  | {
      kind: "indeterminate";
      code: "post_replace_sync_failed";
      attemptedRevision: Revision;
      observedRevision: Revision | null;
      requiresReload: true;
      fieldErrors: [];
    };

export type ResetScope = { kind: "app" } | { kind: "section"; section: SettingsSection };

export type RetryEffectResult = {
  outcome: "applied" | "pending" | "superseded";
  snapshot: ReadySnapshot;
};

export const WORKSPACE_APPEARANCE_SOURCE = "omniget.workspace.appearance.v1";
export type ImportAppearanceResult = {
  outcome: "imported" | "already_imported";
  snapshot: ReadySnapshot;
};

export type AppearanceMode = "system" | "light" | "dark";

export type TypographySettings = {
  font_display: string;
  font_body: string;
  font_mono: string;
  line_height_base: number;
  spacing_scale: number;
  preset_name: string | null;
};

export type SettingsPublicV3 = {
  schema_version: number;
  appearance: {
    theme: string;
    language: string;
    sidebar_collapsed?: boolean;
    mode: AppearanceMode;
    preset: string;
    legacy_theme: string | null;
    migration_version: number;
  };
  download: {
    default_output_dir: string;
    saved_output_dirs: string[];
    always_ask_path: boolean;
    video_quality: string;
    skip_existing: boolean;
    embed_metadata: boolean;
    embed_thumbnail: boolean;
    playlist_number_files: boolean;
    video_compatibility: "best" | "compatible";
    write_nfo_sidecar: boolean;
    clipboard_detection: boolean;
    auto_download_on_paste: boolean;
    filename_template: string;
    organize_by_platform: boolean;
    download_subtitles: boolean;
    include_auto_subtitles: boolean;
    caption_locale: string;
    keep_vtt: boolean;
    translate_metadata: boolean;
    youtube_sponsorblock: boolean;
    sponsorblock_mode: string;
    sponsorblock_categories: string[];
    split_by_chapters: boolean;
    live_from_start: boolean;
    speed_limit: string;
    hotkey_enabled: boolean;
    hotkey_binding: string;
    music_hotkey_enabled: boolean;
    music_hotkey_binding: string;
    music_audio_format: string;
    copy_to_clipboard_on_hotkey: boolean;
    cookie_file: string;
    always_use_managed_cookies: boolean;
    bilibili_danmaku_enabled: boolean;
    bilibili_danmaku_format: string;
    bilibili_container: string;
    bilibili_nfo_enabled: boolean;
    bilibili_cover_sidecar: boolean;
    bilibili_cover_format: string;
    bilibili_naming_video: string;
    bilibili_naming_multi_part: string;
    bilibili_naming_bangumi: string;
    bilibili_naming_cheese: string;
    bilibili_naming_collection: string;
    bilibili_cdn_hosts: string;
    bilibili_cdn_prefer_alternatives: boolean;
    bilibili_preferred_qn: number;
    bilibili_preferred_codec: number;
    bilibili_preferred_audio_qn: number;
  };
  proxy?: {
    enabled?: boolean;
    proxy_type?: string;
    host?: string;
    port?: number;
    username?: string;
    has_password: boolean;
  };
  advanced: {
    max_concurrent_segments: number;
    max_retries: number;
    max_concurrent_downloads: number;
    concurrent_fragments: number;
    stagger_delay_ms: number;
    torrent_listen_port: number;
    torrent_auto_trackers: boolean;
    torrent_upnp: boolean;
    torrent_stop_seeding: boolean;
    prevent_sleep: boolean;
    cookies_from_browser: string;
    has_twitter_manual_cookie: boolean;
    user_agent: string;
    insecure_tls?: boolean;
  };
  telegram: {
    concurrent_downloads: number;
    fix_file_extensions: boolean;
    enabled?: boolean;
  };
  rpc: {
    enabled: boolean;
    app_id: string;
    large_image_key: string;
  };
  bridge?: {
    enabled?: boolean;
    port?: number;
    has_token: boolean;
  };
  llm?: {
    turn_idle_timeout_s?: number;
    turn_tool_idle_timeout_s?: number;
    turn_start_timeout_s?: number;
    job_default_deadline_min?: number;
    ask_timeout_chat_s?: number;
    ask_timeout_job_s?: number | null;
    approval_notify_delay_s?: number;
    jobs_max_parallel?: number;
    jobs_max_per_agent?: number | null;
  };
  world?: {
    enabled?: boolean;
  };
  canvas?: Record<string, unknown>;
  music?: {
    enabled?: boolean;
  };
  video?: {
    enabled?: boolean;
  };
  tube?: Partial<import("$lib/tube/types").TubeSettings>;
  washmybelly?: {
    enabled?: boolean;
    quarantine_days?: number;
    protected_paths_extra?: string[];
    auto_hold_power?: boolean;
    perf_tool_path?: string;
  };
  superpowers?: {
    pinned?: string[];
  };
  read?: {
    enabled?: boolean;
  };
  league?: {
    enabled?: boolean;
    auto_accept?: boolean;
    auto_accept_delay?: number;
    notify_ready_check?: boolean;
    auto_pick?: boolean;
    auto_ban?: boolean;
    auto_ban_delay?: number;
    auto_lock?: boolean;
    auto_lock_at_timeout?: boolean;
    auto_runes?: boolean;
    auto_honor?: boolean;
    auto_play_again?: boolean;
    auto_requeue?: boolean;
    auto_accept_swaps?: boolean;
    auto_reconnect?: boolean;
    auto_trade?: string;
    auto_message?: string;
    pick_champions?: number[];
    ban_champions?: number[];
    pick_random?: boolean;
    skin_roulette?: boolean;
    skin_roulette_include_base?: boolean;
    ward_roulette?: boolean;
    sgp_enabled?: boolean;
    coach_style?: "objective" | "roast" | "praise";
  };
  onboarding_completed: boolean;
  start_with_system: boolean;
  start_minimized: boolean;
  exit_on_close: boolean;
  run_in_background: boolean;
  portable_mode?: boolean;
  legal_acknowledged?: boolean;
  last_download_options?: {
    mode?: "auto" | "audio" | "mute" | null;
    quality?: string | null;
  };
  accessibility?: {
    reduce_motion?: boolean;
    reduce_transparency?: boolean;
  };
  typography?: TypographySettings;
};

const REVISION_PATTERN = /^(0|[1-9]\d{0,19})$/;
const U64_MAX = 18446744073709551615n;

export function isRevision(value: unknown): value is Revision {
  return typeof value === "string" && REVISION_PATTERN.test(value) && BigInt(value) <= U64_MAX;
}

export function compareRevision(a: Revision, b: Revision): number {
  const left = BigInt(a);
  const right = BigInt(b);
  return left < right ? -1 : left > right ? 1 : 0;
}

const API_ERROR_KINDS = new Set(["validation", "conflict", "unavailable", "persistence", "indeterminate"]);

export function isApiError(value: unknown): value is ApiError {
  return typeof value === "object" && value !== null
    && API_ERROR_KINDS.has((value as { kind?: unknown }).kind as string)
    && typeof (value as { code?: unknown }).code === "string";
}
