import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "aria2-remote",
  titleKey: "downloads.tools.aria2_remote.title",
  descriptionKey: "downloads.tools.aria2_remote.description",
  icon: "M4 14h16v6H4zM8 17h.01M12 3v8M8.5 7.5 12 11l3.5-3.5",
  order: 70,
  load: () => import("./Aria2RemoteTool.svelte"),
};
