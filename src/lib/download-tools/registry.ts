import type { Component } from "svelte";

/**
 * A tool shown in Downloads › Tools.
 *
 * Each tool lives in its own `src/components/download/tools/<id>.tool.ts`
 * and is picked up here, so adding one never touches a shared list.
 */
export interface DownloadTool {
  /** Stable id, also used in `?tool=<id>` links. */
  id: string;
  /** i18n key of the card title, under `downloads.tools.<id>`. */
  titleKey: string;
  /** i18n key of the one-line description shown on the card. */
  descriptionKey: string;
  /** Path data for a 24×24 stroke icon drawn with `currentColor`. */
  icon: string;
  /** Lower comes first; ties sort by id. */
  order?: number;
  /** Loads the panel shown when the card is opened. */
  load: () => Promise<{ default: Component }>;
}

const modules = import.meta.glob<{ tool: DownloadTool }>("/src/components/download/tools/*.tool.ts", {
  eager: true,
});

export const DOWNLOAD_TOOLS: DownloadTool[] = Object.values(modules)
  .map((m) => m.tool)
  .filter(Boolean)
  .sort((a, b) => (a.order ?? 100) - (b.order ?? 100) || a.id.localeCompare(b.id));

export function findDownloadTool(id: string | null | undefined): DownloadTool | undefined {
  return id ? DOWNLOAD_TOOLS.find((tool) => tool.id === id) : undefined;
}
