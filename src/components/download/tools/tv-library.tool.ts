import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "tv-library",
  titleKey: "downloads.tools.tv_library.title",
  descriptionKey: "downloads.tools.tv_library.description",
  icon: "M3 7h18v12H3zM8 3l4 4 4-4",
  order: 47,
  load: () => import("./TvLibraryTool.svelte"),
};
