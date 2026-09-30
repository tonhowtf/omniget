/**
 * What a deep link does once its destination page has parsed it (the parsers
 * live in `$lib/settings-navigation`). Kept pure and dependency-injected so the
 * rule "navigation only reveals and focuses, it never mutates" is testable:
 * nothing here can invoke a command.
 */
import type { parseMcpTarget, parseMusicTarget, parseRosterTarget } from "$lib/settings-navigation";

type McpMode = ReturnType<typeof parseMcpTarget>["mode"];
type McpSection = ReturnType<typeof parseMcpTarget>["section"];
type MusicTarget = ReturnType<typeof parseMusicTarget>;
type RosterSection = ReturnType<typeof parseRosterTarget>["section"];

/**
 * The href `replaceState` should move to when the MCP half selector changes,
 * or null when the URL already says so. Client is the default, so it drops
 * `mode` (and the server-only `section`); other params and the hash stay.
 */
export function mcpModeHref(url: URL, mode: McpMode): string | null {
  const next = new URL(url);
  if (mode === "server") next.searchParams.set("mode", "server");
  else {
    next.searchParams.delete("mode");
    next.searchParams.delete("section");
  }
  const href = `${next.pathname}${next.search}${next.hash}`;
  return href === `${url.pathname}${url.search}${url.hash}` ? null : href;
}

/** DOM ids of the MCP server sections a link can focus. */
export const MCP_SECTION_IDS = { permissions: "mcp-section-permissions", media: "mcp-section-media" } as const;

export function mcpSectionSelector(section: McpSection): string | null {
  return section ? `#${MCP_SECTION_IDS[section]}` : null;
}

/** DOM ids of the blocks of the selected bot a link can focus. */
export const ROSTER_SECTION_IDS = { skills: "bot-section-skills", learning: "bot-section-learning" } as const;

export function rosterSectionSelector(section: RosterSection): string | null {
  return section ? `#${ROSTER_SECTION_IDS[section]}` : null;
}

/**
 * Reveal the Music settings view and/or the equalizer through the existing
 * navigation (`go`, `ui.side`). Returns the selector to focus, or null.
 * Never plays, scans, adds folders or enables Music.
 */
export function applyMusicTarget(
  target: MusicTarget,
  deps: { currentKind: string; go: (view: { kind: "settings" }) => void; showEq: () => void },
): string | null {
  if (target.view === "settings" && deps.currentKind !== "settings") deps.go({ kind: "settings" });
  if (target.panel === "eq") {
    deps.showEq();
    return "#music-eq-panel";
  }
  return target.view === "settings" ? "#music-settings-view" : null;
}
