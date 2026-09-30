import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "loudness",
  titleKey: "downloads.tools.loudness.title",
  descriptionKey: "downloads.tools.loudness.description",
  // A waveform whose peaks meet one line.
  icon: "M3 12h2M7 8v8M11 5v14M15 8v8M19 10v4M21 12h0",
  order: 45,
  load: () => import("./LoudnessTool.svelte"),
};
