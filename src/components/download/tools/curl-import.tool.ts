import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "curl-import",
  titleKey: "downloads.tools.curl_import.title",
  descriptionKey: "downloads.tools.curl_import.description",
  icon: "M4 17l6-5-6-5M12 19h8",
  order: 25,
  load: () => import("./CurlImportTool.svelte"),
};
