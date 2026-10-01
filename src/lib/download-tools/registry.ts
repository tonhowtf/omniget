import type { Component } from "svelte";

export interface DownloadTool {
  id: string;
  titleKey: string;
  descriptionKey: string;
  icon: string;
  order?: number;
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
