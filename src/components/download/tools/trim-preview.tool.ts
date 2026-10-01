import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "trim-preview",
  titleKey: "downloads.tools.trim_preview.title",
  descriptionKey: "downloads.tools.trim_preview.description",
  icon: "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12",
  order: 35,
  load: () => import("./TrimPreviewTool.svelte"),
};
