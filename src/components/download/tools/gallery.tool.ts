import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "gallery",
  titleKey: "downloads.tools.gallery.title",
  descriptionKey: "downloads.tools.gallery.description",
  // A picture frame with a mountain.
  icon: "M3 5h18v14H3zM3 16l5-5 4 4 3-3 6 6M15.5 9.5h.01",
  order: 25,
  load: () => import("./GalleryTool.svelte"),
};
