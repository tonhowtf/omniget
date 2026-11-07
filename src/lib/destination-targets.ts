import type { parseMusicTarget } from "$lib/settings-navigation";

type MusicTarget = ReturnType<typeof parseMusicTarget>;

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
