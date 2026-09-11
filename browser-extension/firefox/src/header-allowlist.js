// The old blocklist let anything unrecognised through, which meant shipping
// fingerprinting and client-hint headers to the desktop app for no reason. An
// allowlist inverts the default: a header travels only if it is something the
// download actually needs to succeed.
export const ALLOWED_HEADERS = Object.freeze([
  "referer",
  "origin",
  "cookie",
  "authorization",
  "auth",
  "token",
  "key",
  "access-token",
  "api-key",
  "app-token",
  "authtoken",
  "session-id",
]);

// Vendors invent their own auth headers constantly; these are the ones worth
// carrying without having to enumerate every CDN by hand.
const ALLOWED_X_HEADER = /(auth|token|sign|key|ticket|session)/i;

export function isAllowedHeader(name) {
  if (typeof name !== "string") return false;
  const lower = name.trim().toLowerCase();
  if (!lower) return false;
  if (ALLOWED_HEADERS.includes(lower)) return true;
  return lower.startsWith("x-") && ALLOWED_X_HEADER.test(lower);
}

// Takes the webRequest `requestHeaders` shape ([{ name, value }]) and returns a
// plain object ready to be put on the bridge payload, or null when nothing
// survived the filter.
export function extractSendableHeaders(requestHeaders) {
  if (!Array.isArray(requestHeaders)) return null;
  const out = {};
  for (const header of requestHeaders) {
    if (!header || typeof header.name !== "string") continue;
    if (typeof header.value !== "string" || header.value === "") continue;
    if (!isAllowedHeader(header.name)) continue;
    out[header.name] = header.value;
  }
  return Object.keys(out).length > 0 ? out : null;
}
