import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "join-videos",
  titleKey: "downloads.tools.join_videos.title",
  descriptionKey: "downloads.tools.join_videos.description",
  icon: "M3 7h7v10H3zM14 7h7v10h-7zM10 12h4",
  order: 36,
  load: () => import("./JoinVideosTool.svelte"),
};
