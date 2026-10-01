import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "size-check",
  titleKey: "downloads.tools.size_check.title",
  descriptionKey: "downloads.tools.size_check.description",
  icon: "M3 13h18v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-6zM3 13l3-8h12l3 8M7 17h.01M11 17h6",
  order: 30,
  load: () => import("./SizeCheckTool.svelte"),
};
