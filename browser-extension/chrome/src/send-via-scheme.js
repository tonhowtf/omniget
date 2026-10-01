export const OMNIGET_SCHEME = "omniget://";
const SCHEME_TAB_LIFETIME_MS = 1500;

export function buildOmnigetSchemeUrl(url) {
  if (typeof url !== "string") {
    return null;
  }
  const trimmed = url.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.startsWith(OMNIGET_SCHEME)) {
    return trimmed;
  }
  if (trimmed.startsWith("magnet:") || trimmed.startsWith("p2p:")) {
    return `omniget:${trimmed}`;
  }
  const withoutProtocol = trimmed.replace(/^https?:\/\//i, "");
  if (!withoutProtocol) {
    return null;
  }
  return `${OMNIGET_SCHEME}${withoutProtocol}`;
}

export async function openOmnigetScheme(
  url,
  {
    tabs = chrome?.tabs,
    schedule = (fn, ms) => setTimeout(fn, ms),
    tabLifetimeMs = SCHEME_TAB_LIFETIME_MS,
  } = {}
) {
  const schemeUrl = buildOmnigetSchemeUrl(url);
  if (!schemeUrl) {
    return { ok: false, reason: "invalid-url" };
  }
  if (!tabs?.create) {
    return { ok: false, reason: "tabs-api-unavailable" };
  }
  try {
    const newTab = await tabs.create({ url: schemeUrl, active: false });
    const tabId = newTab?.id;
    if (typeof tabId === "number" && typeof tabs.remove === "function") {
      schedule(() => {
        try {
          const removeResult = tabs.remove(tabId);
          if (removeResult && typeof removeResult.catch === "function") {
            removeResult.catch(() => {});
          }
        } catch {
        }
      }, tabLifetimeMs);
    }
    return { ok: true, schemeUrl };
  } catch (error) {
    return {
      ok: false,
      reason: "tabs-create-failed",
      message: error instanceof Error ? error.message : String(error),
    };
  }
}
