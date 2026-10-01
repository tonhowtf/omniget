import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "link-export",
  titleKey: "downloads.tools.link_export.title",
  descriptionKey: "downloads.tools.link_export.description",
  icon: "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6M9 15h6M12 12l3 3-3 3",
  order: 55,
  load: () => import("./LinkExportTool.svelte"),
};
