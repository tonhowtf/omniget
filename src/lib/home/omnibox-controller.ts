export type OmniState =
  | { kind: "idle" }
  | { kind: "detecting" }
  | { kind: "detected"; info: PlatformInfo }
  | { kind: "unsupported" }
  | { kind: "preparing"; platform: string }
  | { kind: "batch"; urls: string[] }
  | { kind: "searching" }
  | { kind: "search-results"; results: SearchResult[] }
  | { kind: "search-empty" }
  | { kind: "error"; message: string; originalUrl: string; platform: string };

export type PlatformInfo = {
  platform: string;
  supported: boolean;
  content_id: string | null;
  content_type: string | null;
};

export type SearchResult = {
  id: string;
  title: string;
  author: string;
  duration: number | null;
  thumbnail_url: string | null;
  url: string;
  platform: string;
};

export type HomeInputMode = "url" | "batch" | "torrent" | "p2p";

export function isUrl(value: string): boolean {
  return (
    value.startsWith("http://") ||
    value.startsWith("https://") ||
    value.startsWith("magnet:") ||
    value.startsWith("p2p:") ||
    value.endsWith(".torrent")
  );
}

export function showInspectorForState(state: OmniState): boolean {
  return (
    state.kind === "detected" ||
    state.kind === "preparing" ||
    state.kind === "error"
  );
}

export function showOmniboxForState(state: OmniState): boolean {
  return (
    state.kind === "idle" ||
    state.kind === "detecting" ||
    state.kind === "detected" ||
    state.kind === "unsupported" ||
    state.kind === "batch" ||
    state.kind === "searching" ||
    state.kind === "search-results" ||
    state.kind === "search-empty"
  );
}
