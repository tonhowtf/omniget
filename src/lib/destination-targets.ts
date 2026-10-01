import type { parseMcpTarget, parseMusicTarget, parseRosterTarget } from "$lib/settings-navigation";

type McpMode = ReturnType<typeof parseMcpTarget>["mode"];
type McpSection = ReturnType<typeof parseMcpTarget>["section"];
type MusicTarget = ReturnType<typeof parseMusicTarget>;
type RosterSection = ReturnType<typeof parseRosterTarget>["section"];

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

export const MCP_SECTION_IDS = { permissions: "mcp-section-permissions", media: "mcp-section-media" } as const;

export function mcpSectionSelector(section: McpSection): string | null {
  return section ? `#${MCP_SECTION_IDS[section]}` : null;
}

export const ROSTER_SECTION_IDS = { skills: "bot-section-skills", learning: "bot-section-learning" } as const;

export function rosterSectionSelector(section: RosterSection): string | null {
  return section ? `#${ROSTER_SECTION_IDS[section]}` : null;
}

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
