// A playlist that only ever existed as a `blob:` has no address the desktop app
// could fetch, so deep search gives it an identity instead of a location and
// sends the playlist text alongside. The host is a reserved `.invalid` name
// (RFC 2606): it can never resolve, which is the point — if anything ever tries
// to fetch one of these, that is a bug and it should fail loudly.
//
// deep-search.js runs in the page as a plain script and cannot import this
// module, so it repeats the host as a literal. A test keeps the two in step.
export const SYNTHETIC_MANIFEST_HOST = "deep-search.omniget.invalid";

export function isSyntheticManifestUrl(url) {
  try {
    return new URL(url).hostname === SYNTHETIC_MANIFEST_HOST;
  } catch {
    return false;
  }
}
