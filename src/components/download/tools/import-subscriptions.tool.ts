import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "import-subscriptions",
  titleKey: "downloads.tools.import_subscriptions.title",
  descriptionKey: "downloads.tools.import_subscriptions.description",
  // An RSS feed: a dot and two arcs.
  icon: "M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16M5 20a1 1 0 1 0 0-2 1 1 0 0 0 0 2z",
  order: 30,
  load: () => import("./ImportSubscriptionsTool.svelte"),
};
