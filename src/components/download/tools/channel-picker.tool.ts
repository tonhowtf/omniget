import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "channel-picker",
  titleKey: "downloads.tools.channel_picker.title",
  descriptionKey: "downloads.tools.channel_picker.description",
  icon: "M9 6h11M9 12h11M9 18h11M4 6l1 1 2-2M4 12l1 1 2-2M4 18l1 1 2-2",
  order: 35,
  load: () => import("./ChannelPickerTool.svelte"),
};
