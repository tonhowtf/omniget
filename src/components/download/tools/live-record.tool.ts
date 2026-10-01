import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "live-record",
  titleKey: "downloads.tools.live_record.title",
  descriptionKey: "downloads.tools.live_record.description",
  icon: "M12 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM16.24 7.76a6 6 0 0 1 0 8.48M7.76 16.24a6 6 0 0 1 0-8.48M19.07 4.93a10 10 0 0 1 0 14.14M4.93 19.07a10 10 0 0 1 0-14.14",
  order: 45,
  load: () => import("./LiveRecordTool.svelte"),
};
