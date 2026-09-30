import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "search-sites",
  titleKey: "downloads.tools.search_sites.title",
  descriptionKey: "downloads.tools.search_sites.description",
  // A magnifying glass.
  icon: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-4.35-4.35",
  order: 15,
  load: () => import("./SearchSitesTool.svelte"),
};
