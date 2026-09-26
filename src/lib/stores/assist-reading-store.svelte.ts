/**
 * Reading companion (`assist::reading`): journeys, progress, rounds of film
 * recommendations with their access evidence, and the person's streaming
 * preferences. Wire shapes mirror `src-tauri/src/commands/assist/reading.rs`.
 *
 * Nothing runs at rest: every read is one `invoke` fired by a mount or a
 * click. A failed write returns an error key and keeps the caller's draft.
 */
import { invoke } from "@tauri-apps/api/core";

// ── Wire types ──────────────────────────────────────────────────────────

export type JourneyStatus = "active" | "paused" | "finished" | "abandoned";
export type PositionKind = "chapter" | "page" | "percent" | "locator";
export type AccessKind = "subscription" | "rent" | "buy" | "free";
export type EvidenceStatus = "confirmed" | "inferred" | "absent" | "unknown";
export type FinalStatus = "confirmed" | "probable" | "unconfirmed";
export type Role = "entry" | "shift" | "surprise";
export type ViewingKind = "recommended" | "planned" | "watched" | "abandoned" | "declined";

export interface Journey {
  id: string;
  bot_id: string;
  scope: string;
  title: string;
  author?: string | null;
  edition?: string | null;
  language?: string | null;
  external_id?: string | null;
  status: JourneyStatus;
  spoiler_terms: string[];
  notes?: string | null;
  started_ms: number;
  finished_ms?: number | null;
  updated_ms: number;
}

export interface Progress {
  id: string;
  journey_id: string;
  position_kind: PositionKind;
  value: string;
  value_num?: number | null;
  edition?: string | null;
  origin: "manual" | "reader" | "bot";
  note?: string | null;
  recorded_ms: number;
}

export interface RoundContext {
  id: string;
  text: string;
  created_ms: number;
  expires_ms: number;
}

export interface AccessPrefs {
  bot_id: string;
  region: string;
  subscriptions: string[];
  access_kinds: AccessKind[];
  only_confirmed: boolean;
  freshness_days: number;
  updated_ms: number;
}

export interface Movie {
  id: string;
  title: string;
  original_title?: string | null;
  year: number;
  director?: string | null;
}

export interface Availability {
  id: string;
  movie_id: string;
  region: string;
  platform: string;
  access_kind: AccessKind;
  url?: string | null;
  status: EvidenceStatus;
  source_kind: string;
  quote?: string | null;
  note?: string | null;
  checked_ms: number;
  page_date_ms?: number | null;
}

export interface Subtitle {
  id: string;
  language: string;
  kind: "subtitle" | "audio";
  status: EvidenceStatus;
  url?: string | null;
  source_kind: string;
  quote?: string | null;
  login_required: boolean;
  note?: string | null;
  checked_ms: number;
}

export interface ViewingEvent {
  id: string;
  movie_id: string;
  kind: ViewingKind;
  reaction?: string | null;
  reasons: string[];
  origin: string;
  created_ms: number;
}

export interface RoundItem {
  id: string;
  movie_id: string;
  role: Role;
  connection: string;
  why: string;
  moods: string[];
  pace: "easy" | "attentive";
  heavy: boolean;
  resumes_item_id?: string | null;
  questions: string[];
  status: "confirmed" | "probable";
  missing: string[];
}

export interface ItemView {
  item: RoundItem;
  movie: Movie;
  recorded_status: "confirmed" | "probable";
  availability?: Availability | null;
  subtitle?: Subtitle | null;
  current: { status: FinalStatus; reasons: string[] };
  why_evidence: { event_id: string; movie: Movie; kind: ViewingKind; reaction?: string | null; reasons: string[]; created_ms: number }[];
  state?: ViewingKind | null;
}

export interface RoundView {
  round: {
    id: string;
    reason?: string | null;
    context_text?: string | null;
    shortfall_reason?: string | null;
    only_confirmed: boolean;
    skill_name?: string | null;
    skill_hash?: string | null;
    look_for: { movie_id: string; title: string; year: number; note?: string | null; status: FinalStatus; reasons: string[] }[];
    created_ms: number;
  };
  items: ItemView[];
}

export interface JourneyDetail {
  journey: Journey;
  progress?: Progress | null;
  progress_history: Progress[];
  contexts: RoundContext[];
  prefs: AccessPrefs;
  films: { movie: Movie; state: ViewingKind; watched: boolean; events: ViewingEvent[] }[];
  rounds: RoundView[];
  spoiler_terms_count: number;
}

export interface SkillStatus {
  name: string;
  installed: boolean;
  differs: boolean;
}

export interface Overview {
  journeys: JourneyDetail[];
  prefs: AccessPrefs;
  skill: SkillStatus | null;
  /** The curation skill is already linked to this bot. */
  skill_bound?: boolean;
}

export type Outcome<T = void> = { ok: true; value: T } | { ok: false; errorKey: string; detail: string };

// ── Pure helpers ────────────────────────────────────────────────────────

export const POSITION_KINDS: PositionKind[] = ["chapter", "page", "percent", "locator"];
export const ACCESS_KINDS: AccessKind[] = ["subscription", "free", "rent", "buy"];
export const JOURNEY_STATUSES: JourneyStatus[] = ["active", "paused", "finished", "abandoned"];

export function readingErrorKey(err: unknown): string {
  const s = String((err as { message?: string })?.message ?? err ?? "");
  switch (s.split(":")[0]?.trim()) {
    case "ERR_READING_INPUT":
      return "assist.reading.err_input";
    case "ERR_READING_NOT_FOUND":
      return "assist.reading.err_not_found";
    case "ERR_READING_SCOPE":
      return "assist.reading.err_scope";
    case "ERR_READING_SKILL":
      return "assist.reading.err_skill";
    case "ERR_ASSIST_DB":
      return "assist.reading.err_db";
    default:
      return "assist.reading.err_generic";
  }
}

/** The message after the code, for "advanced details". */
export function errorDetail(err: unknown): string {
  const s = String((err as { message?: string })?.message ?? err ?? "");
  const i = s.indexOf(":");
  return i > 0 && s.slice(0, i).startsWith("ERR_") ? s.slice(i + 1).trim() : s;
}

export function statusKey(s: FinalStatus | EvidenceStatus): string {
  return `assist.reading.status_${s}`;
}

export function roleKey(r: Role): string {
  return `assist.reading.role_${r}`;
}

export function moodKey(m: string): string {
  return `assist.reading.mood_${m}`;
}

export function accessKey(k: AccessKind): string {
  return `assist.reading.access_${k}`;
}

export function viewingKey(k: ViewingKind): string {
  return `assist.reading.viewing_${k}`;
}

export function positionKey(k: PositionKind): string {
  return `assist.reading.position_${k}`;
}

/** "chapter 3", "42%", "page 120" — value only; the kind goes through i18n. */
export function progressValue(p: Pick<Progress, "position_kind" | "value">): string {
  return p.position_kind === "percent" ? `${p.value.replace(/%$/, "")}%` : p.value;
}

/** Only http(s) links are rendered as links. */
export function safeHref(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:" ? u.toString() : null;
  } catch {
    return null;
  }
}

/** Splits "a, b; c" into trimmed terms (spoiler terms, subscriptions). */
export function splitList(s: string): string[] {
  return s
    .split(/[,;\n]/)
    .map((x) => x.trim())
    .filter((x) => x.length > 0);
}

// ── State ───────────────────────────────────────────────────────────────

const state = $state({
  byBot: {} as Record<string, Overview>,
  loading: false,
  errorKey: null as string | null,
});

export function getOverview(botId: string): Overview | null {
  return state.byBot[botId] ?? null;
}

export function isReadingLoading(): boolean {
  return state.loading;
}

export function getReadingErrorKey(): string | null {
  return state.errorKey;
}

export function resetReadingStore(): void {
  state.byBot = {};
  state.loading = false;
  state.errorKey = null;
}

export async function loadOverview(botId: string): Promise<void> {
  if (!botId) return;
  state.loading = true;
  try {
    state.byBot[botId] = await invoke<Overview>("assist_reading_overview", { botId });
    state.errorKey = null;
  } catch (e) {
    state.errorKey = readingErrorKey(e);
  } finally {
    state.loading = false;
  }
}

async function act<T>(botId: string, cmd: string, args: Record<string, unknown>): Promise<Outcome<T>> {
  try {
    const value = await invoke<T>(cmd, args);
    await loadOverview(botId);
    return { ok: true, value };
  } catch (e) {
    return { ok: false, errorKey: readingErrorKey(e), detail: errorDetail(e) };
  }
}

export function startJourney(
  botId: string,
  journey: { title: string; author?: string; edition?: string; language?: string; external_id?: string; spoiler_terms?: string[] },
) {
  return act<Journey>(botId, "assist_reading_start_journey", { botId, journey });
}

export function updateJourney(botId: string, journeyId: string, patch: Partial<Omit<Journey, "id">>) {
  return act<Journey>(botId, "assist_reading_update_journey", { journeyId, patch });
}

export function deleteJourney(botId: string, journeyId: string) {
  return act<void>(botId, "assist_reading_delete_journey", { journeyId });
}

export function recordProgress(
  botId: string,
  journeyId: string,
  kind: PositionKind,
  value: string,
  edition?: string,
  origin: "manual" | "reader" = "manual",
) {
  return act<{ progress: Progress; not_comparable?: string | null }>(botId, "assist_reading_record_progress", {
    journeyId,
    kind,
    value,
    edition: edition?.trim() || null,
    origin,
    note: null,
  });
}

export function noteContext(botId: string, journeyId: string, text: string, hours?: number) {
  return act<RoundContext>(botId, "assist_reading_note_context", { journeyId, text, hours: hours ?? null });
}

export function recordViewing(botId: string, journeyId: string, movieId: string, kind: Exclude<ViewingKind, "recommended">, reaction?: string) {
  return act<ViewingEvent>(botId, "assist_reading_record_viewing", {
    journeyId,
    movieId,
    kind,
    reaction: reaction?.trim() || null,
  });
}

export function forgetReaction(botId: string, eventId: string) {
  return act<void>(botId, "assist_reading_forget_reaction", { eventId });
}

export function setPrefs(botId: string, patch: Partial<Omit<AccessPrefs, "bot_id" | "updated_ms">>) {
  return act<AccessPrefs>(botId, "assist_reading_set_prefs", { botId, patch });
}

export function installSkill(botId: string, replace = false) {
  return act<{ already_installed: boolean; outcome?: { needs_confirm?: string | null } }>(botId, "assist_reading_install_skill", { replace });
}

