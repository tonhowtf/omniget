import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "page-files",
  titleKey: "downloads.tools.page_files.title",
  descriptionKey: "downloads.tools.page_files.description",
  icon: "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h5M14 3l5 5M14 3v5h5M19 8v3M17.5 17.5m-3 0a3 3 0 1 0 6 0a3 3 0 1 0-6 0M22 22l-2.4-2.4",
  order: 32,
  load: () => import("./PageFilesTool.svelte"),
};
