import { ANIMATION_CLIP, type Mood } from './pet-state';
import { asEntry, entrySuggestions, entrySummary } from '$lib/canvas/companion/api';

const BALLOON_TEXT_MAX = 110;
const BALLOON_CHIP_MAX = 42;

export interface CompanionBalloon {
  id: string;
  name: string;
  text: string;
  chips: string[];
  mood: Mood;
  clip: string;
  ms: number;
  via?: string;
}

function clip(s: string, max: number): string {
  const t = s.replace(/\s+/g, ' ').trim();
  return t.length <= max ? t : `${t.slice(0, max - 1)}…`;
}

function viaOf(src: unknown): string | undefined {
  if (typeof src !== 'string' || !src || src === 'heuristic') return undefined;
  if (src.startsWith('via ')) return src;
  if (src.startsWith('ollama:')) return `via ${src.slice(7)}`;
  if (src.startsWith('llm:')) return `via ${src.slice(4)}`;
  return undefined;
}

export function balloonOf(
  raw: unknown,
  tr?: (key: string, vars?: Record<string, unknown>) => string,
): CompanionBalloon | null {
  const e = raw as {
    id?: unknown;
    name?: unknown;
    summary?: unknown;
    suggestions?: unknown;
    kind?: unknown;
    celebrate?: unknown;
    petBalloon?: unknown;
    source?: unknown;
  } | null;
  if (!e || e.petBalloon !== true) return null;
  if (typeof e.id !== 'string' || typeof e.summary !== 'string') return null;
  const entry = tr ? asEntry(raw) : null;
  const localized = entry?.i18n && tr ? { summary: entrySummary(entry, tr), suggestions: entrySuggestions(entry, tr) } : null;
  const suggestions: unknown = localized?.suggestions ?? e.suggestions;
  const chips = Array.isArray(suggestions)
    ? suggestions
        .filter((s): s is string => typeof s === 'string' && s.trim().length > 0)
        .slice(0, 3)
        .map((s) => clip(s, BALLOON_CHIP_MAX))
    : [];
  const kind = String(e.kind ?? '');
  const celebrate = e.celebrate === true;
  const mood: Mood =
    kind === 'failed' ? 'Worried' : kind === 'blocked' ? 'Curious' : celebrate ? 'Happy' : 'Neutral';
  const clipName =
    kind === 'failed'
      ? ANIMATION_CLIP.Worried
      : kind === 'blocked'
        ? ANIMATION_CLIP.Wave
        : celebrate
          ? ANIMATION_CLIP.Celebrate
          : ANIMATION_CLIP.Talk;
  return {
    id: e.id,
    name: typeof e.name === 'string' ? clip(e.name, 24) : '',
    text: clip(localized?.summary ?? e.summary, BALLOON_TEXT_MAX),
    chips,
    mood,
    clip: clipName,
    ms: kind === 'blocked' ? 60_000 : 25_000,
    via: viaOf(e.source),
  };
}
