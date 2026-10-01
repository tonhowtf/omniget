export const SYNTHETIC_MANIFEST_HOST = "deep-search.omniget.invalid";

export function isSyntheticManifestUrl(url) {
  try {
    return new URL(url).hostname === SYNTHETIC_MANIFEST_HOST;
  } catch {
    return false;
  }
}
