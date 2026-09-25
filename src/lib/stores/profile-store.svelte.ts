/**
 * Local profile store.
 *
 * The profile is an account-less identity: an ed25519 public key kept by the
 * Rust side (`f1-perfil-core`), plus a nickname and a skin (art id + RGB tint).
 * There is no login, no e-mail and no network call here — every value comes
 * from the five `profile_*` commands.
 *
 * While the Rust side is still a stub every command answers `ERR_STUB`; the
 * store treats that as "unavailable" and renders an empty state instead of
 * crashing. Loading happens once per session (no polling, no interval).
 */
import { invoke } from "@tauri-apps/api/core";

export interface ProfileSkin {
  id: string;
  /** RGB, 0–255. */
  tint: [number, number, number];
}

export interface Profile {
  public_key_b64: string;
  fingerprint: string;
  nickname: string;
  skin: ProfileSkin;
  created_at_ms: number;
}

/** Same bound the Rust side enforces (NICKNAME_MAX_CHARS, counted in code points). */
export const NICKNAME_MAX = 32;

/** Art variant shipped before the F5 placeholder set lands. */
export const DEFAULT_SKIN_ID = "omni-default";

export interface SkinTint {
  /** Tint name; also the i18n suffix (`profile.tint.<id>`). */
  id: string;
  hex: string;
  rgb: [number, number, number];
}

function hexToRgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/**
 * Eight tints from the former Tools catalog palette (the light
 * stop of each gradient), so an avatar sits in the same colour family as the
 * rest of the app.
 */
export const SKIN_TINTS: SkinTint[] = [
  { id: "red", hex: "#FF6B6B" },
  { id: "orange", hex: "#FFB340" },
  { id: "yellow", hex: "#FFD426" },
  { id: "green", hex: "#4CD964" },
  { id: "teal", hex: "#48CFDF" },
  { id: "blue", hex: "#5AA9FF" },
  { id: "purple", hex: "#C77DFF" },
  { id: "pink", hex: "#FF5E7A" },
].map((t) => ({ ...t, rgb: hexToRgb(t.hex) }));

/**
 * Mirrors `DEFAULT_TINT` in `src-tauri/src/profile/store.rs`: a profile created
 * before the picker ever ran carries this tint, and it is deliberately NOT one
 * of the eight swatches. `nearestTint` is what makes the picker still show a
 * selected swatch for it.
 */
export const DEFAULT_TINT: [number, number, number] = [110, 139, 255];

/**
 * Closest swatch by euclidean RGB distance. The backend may hold any tint
 * (its own default, a future roster colour, a hand-edited profile.json), so the
 * picker highlights the nearest swatch instead of showing nothing selected.
 */
export function nearestTint(tint: readonly number[] | null | undefined): SkinTint {
  if (!tint || tint.length !== 3 || tint.some((c) => !Number.isFinite(c))) {
    return nearestTint(DEFAULT_TINT);
  }
  let best = SKIN_TINTS[0];
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const swatch of SKIN_TINTS) {
    const dr = swatch.rgb[0] - tint[0];
    const dg = swatch.rgb[1] - tint[1];
    const db = swatch.rgb[2] - tint[2];
    const distance = dr * dr + dg * dg + db * db;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = swatch;
    }
  }
  return best;
}

/** `[r,g,b]` as a CSS colour; out-of-range or malformed input falls back. */
export function tintToCss(tint: readonly number[] | null | undefined): string {
  if (!tint || tint.length !== 3) return `rgb(${DEFAULT_TINT.join(", ")})`;
  const clamped = tint.map((c) =>
    Number.isFinite(c) ? Math.min(255, Math.max(0, Math.round(c))) : 0,
  );
  return `rgb(${clamped.join(", ")})`;
}

export function sameTint(
  a: readonly number[] | null | undefined,
  b: readonly number[] | null | undefined,
): boolean {
  if (!a || !b || a.length !== 3 || b.length !== 3) return false;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

/**
 * Normalizes a nickname the way the backend does and returns `null` when it
 * would be rejected. Kept pure so the UI can disable Save without a round trip.
 */
export function normalizeNickname(raw: string): string | null {
  if (typeof raw !== "string") return null;
  // Same order as normalize_nickname in src-tauri/src/profile/store.rs: trim
  // and NFC first, then reject control characters (\p{Cc} == Rust's
  // char::is_control), then count CODE POINTS, not UTF-16 units, so 32 emoji
  // are 32 characters on both sides.
  const normalized = raw.normalize("NFC").trim();
  if (/\p{Cc}/u.test(normalized)) return null;
  const length = Array.from(normalized).length;
  if (length < 1 || length > NICKNAME_MAX) return null;
  return normalized;
}

/** Fingerprint in groups of 4, the way a user reads it out loud. */
export function formatFingerprint(fingerprint: string, group = 4): string {
  const clean = (fingerprint ?? "").replace(/[\s:-]/g, "");
  if (!clean) return "";
  return (clean.match(new RegExp(`.{1,${group}}`, "g")) ?? []).join(" ");
}

/** Maps a backend `ERR_*` code to an i18n key. */
export function profileErrorKey(err: unknown): string {
  const code = String(err ?? "");
  if (code.includes("ERR_PROFILE_NICKNAME")) return "profile.err_nickname";
  if (code.includes("ERR_PROFILE_STORE")) return "profile.err_store";
  if (code.includes("ERR_PROFILE_SECRET")) return "profile.err_secret";
  if (code.includes("ERR_PROFILE_SIGN")) return "profile.err_sign";
  if (code.includes("ERR_STUB")) return "profile.err_unavailable";
  return "profile.err_unknown";
}

/** `ERR_STUB` means "core not wired yet", not a real failure. */
export function isUnavailable(err: unknown): boolean {
  return String(err ?? "").includes("ERR_STUB");
}

let profile = $state<Profile | null>(null);
let loading = $state(false);
let errorKey = $state<string | null>(null);
let available = $state(true);
let loadedOnce = false;
let inFlight: Promise<void> | null = null;

export function getProfile(): Profile | null {
  return profile;
}

export function isProfileLoading(): boolean {
  return loading;
}

/** i18n key of the last error, or `null`. */
export function getProfileError(): string | null {
  return errorKey;
}

/** False once a command answered `ERR_STUB`: show the empty state. */
export function isProfileAvailable(): boolean {
  return available;
}

export function clearProfileError(): void {
  errorKey = null;
}

/** Loads once per session unless `force` is set. Never throws. */
export function loadProfile(force = false): Promise<void> {
  if (inFlight) return inFlight;
  if (loadedOnce && !force) return Promise.resolve();
  loading = true;
  errorKey = null;
  inFlight = invoke<Profile>("profile_get")
    .then((p) => {
      profile = p;
      available = true;
    })
    .catch((err) => {
      profile = null;
      available = !isUnavailable(err);
      errorKey = profileErrorKey(err);
    })
    .finally(() => {
      loading = false;
      loadedOnce = true;
      inFlight = null;
    });
  return inFlight;
}

/** Returns true when the backend accepted the nickname. */
export async function setNickname(nick: string): Promise<boolean> {
  const normalized = normalizeNickname(nick);
  if (normalized === null) {
    errorKey = "profile.err_nickname";
    return false;
  }
  errorKey = null;
  try {
    const updated = await invoke<Profile>("profile_set_nickname", { nick: normalized });
    profile = updated ?? (profile ? { ...profile, nickname: normalized } : null);
    available = true;
    return true;
  } catch (err) {
    available = !isUnavailable(err);
    errorKey = profileErrorKey(err);
    return false;
  }
}

/** Returns true when the backend accepted the skin. */
export async function setSkin(
  id: string,
  tint: [number, number, number],
): Promise<boolean> {
  errorKey = null;
  try {
    const updated = await invoke<Profile>("profile_set_skin", { id, tint });
    profile = updated ?? (profile ? { ...profile, skin: { id, tint } } : null);
    available = true;
    return true;
  } catch (err) {
    available = !isUnavailable(err);
    errorKey = profileErrorKey(err);
    return false;
  }
}

/** Copies the fingerprint to the clipboard. Returns true on success. */
export async function copyFingerprint(): Promise<boolean> {
  const fp = profile?.fingerprint;
  if (!fp) return false;
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(formatFingerprint(fp));
    return true;
  } catch {
    return false;
  }
}

/** Test seam: drops the cached profile so a fresh `loadProfile` runs. */
export function resetProfileStore(): void {
  profile = null;
  loading = false;
  errorKey = null;
  available = true;
  loadedOnce = false;
  inFlight = null;
}
