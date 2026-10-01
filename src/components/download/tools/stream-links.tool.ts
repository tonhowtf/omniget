import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "stream-links",
  titleKey: "downloads.tools.stream_links.title",
  descriptionKey: "downloads.tools.stream_links.description",
  icon: "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71",
  order: 20,
  load: () => import("./StreamLinksTool.svelte"),
};
