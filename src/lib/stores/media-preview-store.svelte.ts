export type MediaPreview = {
  url: string;
  title: string;
  author: string;
  thumbnail_url: string | null;
  duration_seconds: number | null;
};

let currentPreview = $state<MediaPreview | null>(null);

export function getMediaPreview(): MediaPreview | null {
  return currentPreview;
}

export function setMediaPreview(preview: MediaPreview | null) {
  currentPreview = preview;
}

export function clearMediaPreview() {
  currentPreview = null;
}
