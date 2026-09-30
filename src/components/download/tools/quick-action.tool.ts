import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "quick-action",
  titleKey: "downloads.tools.quick_action.title",
  descriptionKey: "downloads.tools.quick_action.description",
  // A cursor over a menu line.
  icon: "M4 5h16M4 10h9M4 15h6M14 13l7 3-3 1-1 3-3-7z",
  order: 50,
  load: () => import("./QuickActionTool.svelte"),
};
