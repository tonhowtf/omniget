export type VideoOrListLink = { videoId: string; listId: string; isMix: boolean };

export function videoOrListLink(raw: string): VideoOrListLink | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  const host = url.hostname.replace(/^www\.|^m\.|^music\./i, "").toLowerCase();
  const videoId = host === "youtu.be"
    ? url.pathname.slice(1).split("/")[0]
    : host === "youtube.com"
      ? url.searchParams.get("v")
      : null;
  const listId = url.searchParams.get("list");
  if (!videoId || !listId) return null;
  return { videoId, listId, isMix: /^RD/.test(listId) };
}
