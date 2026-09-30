import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "proxy-check",
  titleKey: "downloads.tools.proxy_check.title",
  descriptionKey: "downloads.tools.proxy_check.description",
  // A globe with a shield-like route.
  icon: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3.6 9h16.8M3.6 15h16.8M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18",
  order: 60,
  load: () => import("./ProxyCheckTool.svelte"),
};
