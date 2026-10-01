import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "profile-sync",
  titleKey: "downloads.tools.profile_sync.title",
  descriptionKey: "downloads.tools.profile_sync.description",
  icon: "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM3 21v-1a6 6 0 0 1 9-5.2M21 14a4 4 0 0 0-7-2.6M14 18a4 4 0 0 0 7 2.6M14 11v2h2M21 21v-2h-2",
  order: 40,
  load: () => import("./ProfileSyncTool.svelte"),
};
