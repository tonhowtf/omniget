import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "dedupe",
  titleKey: "downloads.tools.dedupe.title",
  descriptionKey: "downloads.tools.dedupe.description",
  icon: "M8 3h8l4 4v10a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM4 7v12a2 2 0 0 0 2 2h10M10 11h6M10 15h4",
  order: 90,
  load: () => import("./DedupeTool.svelte"),
};
