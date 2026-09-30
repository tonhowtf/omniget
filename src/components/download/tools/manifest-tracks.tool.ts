import type { DownloadTool } from "$lib/download-tools/registry";

export const tool: DownloadTool = {
  id: "manifest-tracks",
  titleKey: "downloads.tools.manifest_tracks.title",
  descriptionKey: "downloads.tools.manifest_tracks.description",
  // Three stacked layers (video, audio, subtitles).
  icon: "M12 2 2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  order: 46,
  load: () => import("./ManifestTracksTool.svelte"),
};
