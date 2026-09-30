import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "link-router",
  titleKey: "downloads.tools.link_router.title",
  descriptionKey: "downloads.tools.link_router.description",
  // One line that forks into two routes.
  icon: "M3 12h5l4-6h9M12 18h9M8 12l4 6M18 3l3 3-3 3M18 15l3 3-3 3",
  order: 30,
  load: () => import("./LinkRouterTool.svelte"),
};
