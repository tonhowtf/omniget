// Pure helpers of the auto-download rule editor.
import type { AutoRule } from "$lib/tube/types";

export const RULE_QUALITIES = ["best", "1080", "720", "480", "audio"] as const;
export const RULE_SB = ["off", "mark", "remove"] as const;

function randomId(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c?.randomUUID) return c.randomUUID();
  return `rule-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function newRule(name = "", channelIds: string[] = []): AutoRule {
  return {
    id: randomId(),
    name,
    enabled: true,
    channel_ids: channelIds,
    include_words: [],
    exclude_words: [],
    min_duration_s: null,
    max_duration_s: null,
    max_age_days: 7,
    include_shorts: false,
    include_live: false,
    quality: "1080",
    sponsorblock: "off",
    folder: null,
    max_gb: 0,
    keep_days: 0,
    keep_per_channel: 0,
  };
}

/** "a, b\nc" -> ["a", "b", "c"]; trims, drops empties and case-insensitive duplicates. */
export function parseWords(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/[,\n;]/)) {
    const w = raw.trim();
    if (!w) continue;
    const k = w.toLocaleLowerCase();
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(w);
  }
  return out;
}

/** Minutes field (string from an <input>) -> seconds, or null when empty/invalid. */
export function minutesToSeconds(v: string | number | null | undefined): number | null {
  if (v == null || v === "") return null;
  const n = typeof v === "number" ? v : Number(String(v).replace(",", "."));
  if (!Number.isFinite(n) || n <= 0) return null;
  return Math.round(n * 60);
}

export function secondsToMinutes(s: number | null | undefined): string {
  if (!s || s <= 0) return "";
  const m = s / 60;
  return Number.isInteger(m) ? String(m) : m.toFixed(1);
}

type RuleProblem = "name" | "duration" | "numbers";

/** What is wrong with a rule before saving (empty = ok). */
export function ruleProblems(r: AutoRule): RuleProblem[] {
  const out: RuleProblem[] = [];
  if (!r.name.trim()) out.push("name");
  if (r.min_duration_s != null && r.max_duration_s != null && r.max_duration_s > 0 && r.min_duration_s > r.max_duration_s) out.push("duration");
  if ([r.max_gb, r.keep_days, r.keep_per_channel, r.max_age_days ?? 0].some((n) => !Number.isFinite(n) || n < 0)) out.push("numbers");
  return out;
}

/** Normalized copy for saving: trimmed name/words, empty folder -> null. */
export function cleanRule(r: AutoRule): AutoRule {
  return {
    ...r,
    name: r.name.trim(),
    include_words: parseWords(r.include_words.join(",")),
    exclude_words: parseWords(r.exclude_words.join(",")),
    folder: r.folder && r.folder.trim() ? r.folder.trim() : null,
    min_duration_s: r.min_duration_s && r.min_duration_s > 0 ? r.min_duration_s : null,
    max_duration_s: r.max_duration_s && r.max_duration_s > 0 ? r.max_duration_s : null,
    max_age_days: r.max_age_days && r.max_age_days > 0 ? r.max_age_days : null,
  };
}

/** Share of the cap in use, 0..1 (0 when there is no cap). */
export function usageRatio(usedBytes: number, capGb: number): number {
  if (!capGb || capGb <= 0) return 0;
  return Math.min(1, Math.max(0, usedBytes / (capGb * 1024 ** 3)));
}
