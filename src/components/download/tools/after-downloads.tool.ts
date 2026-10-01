import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "after-downloads",
  titleKey: "downloads.tools.after_downloads.title",
  descriptionKey: "downloads.tools.after_downloads.description",
  icon: "M12 3v8M6.3 7.3a8 8 0 1 0 11.4 0",
  order: 60,
  load: () => import("./AfterDownloadsTool.svelte"),
};
