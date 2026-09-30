import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "smart-mode",
  titleKey: "downloads.tools.smart_mode.title",
  descriptionKey: "downloads.tools.smart_mode.description",
  // A lightning bolt: one click, saved choices.
  icon: "M13 2 4 14h7l-1 8 9-12h-7l1-8z",
  order: 10,
  load: () => import("./SmartModeTool.svelte"),
};
