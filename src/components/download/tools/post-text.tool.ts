import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "post-text",
  titleKey: "downloads.tools.post_text.title",
  descriptionKey: "downloads.tools.post_text.description",
  icon: "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6M8 13h8M8 17h5",
  order: 30,
  load: () => import("./PostTextTool.svelte"),
};
