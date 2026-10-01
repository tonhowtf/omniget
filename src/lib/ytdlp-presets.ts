import type { AppSettings } from "$lib/stores/settings-store.svelte";

export type YtdlpPresetId = "fast" | "quality" | "compact" | "music_only";

type YtdlpPreset = {
  id: YtdlpPresetId;
  labelKey: string;
  descKey: string;
  download: {
    video_quality: string;
    embed_metadata: boolean;
    embed_thumbnail: boolean;
    download_subtitles: boolean;
  };
};

export const YTDLP_PRESETS: YtdlpPreset[] = [
  {
    id: "fast",
    labelKey: "settings.download.preset_fast",
    descKey: "settings.download.preset_fast_desc",
    download: {
      video_quality: "480p",
      embed_metadata: false,
      embed_thumbnail: false,
      download_subtitles: false,
    },
  },
  {
    id: "quality",
    labelKey: "settings.download.preset_quality",
    descKey: "settings.download.preset_quality_desc",
    download: {
      video_quality: "best",
      embed_metadata: true,
      embed_thumbnail: true,
      download_subtitles: false,
    },
  },
  {
    id: "compact",
    labelKey: "settings.download.preset_compact",
    descKey: "settings.download.preset_compact_desc",
    download: {
      video_quality: "720p",
      embed_metadata: true,
      embed_thumbnail: false,
      download_subtitles: false,
    },
  },
  {
    id: "music_only",
    labelKey: "settings.download.preset_music_only",
    descKey: "settings.download.preset_music_only_desc",
    download: {
      video_quality: "audio",
      embed_metadata: true,
      embed_thumbnail: true,
      download_subtitles: false,
    },
  },
];

export function matchActivePreset(settings: AppSettings | null): YtdlpPresetId | null {
  if (!settings) return null;
  const d = settings.download;
  for (const preset of YTDLP_PRESETS) {
    const p = preset.download;
    if (
      d.video_quality === p.video_quality
      && d.embed_metadata === p.embed_metadata
      && d.embed_thumbnail === p.embed_thumbnail
      && d.download_subtitles === p.download_subtitles
    ) {
      return preset.id;
    }
  }
  return null;
}
