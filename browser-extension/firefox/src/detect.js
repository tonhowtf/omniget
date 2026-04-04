const MEDIA_CONTENT_TYPES = new Set([
  "audio",
  "clip",
  "course",
  "image",
  "playlist",
  "post",
  "profile",
  "reel",
  "short",
  "video",
]);

export function detectSupportedMediaUrl(rawUrl) {
  if (!rawUrl || typeof rawUrl !== "string") {
    return null;
  }

  if (!(rawUrl.startsWith("http://") || rawUrl.startsWith("https://"))) {
    return null;
  }

  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return null;
  }

  const platform = detectPlatform(parsed);
  if (!platform) {
    return null;
  }

  const contentType = detectContentType(platform, parsed);
  return {
    platform,
    contentType,
    supported: MEDIA_CONTENT_TYPES.has(contentType),
    url: rawUrl,
  };
}

function detectPlatform(url) {
  const host = url.hostname.toLowerCase();

  if (matchesHost(host, "hotmart.com")) return "hotmart";
  if (matchesHost(host, "youtube.com") || matchesHost(host, "youtube-nocookie.com") || host === "youtu.be") return "youtube";
  if (matchesHost(host, "instagram.com") || matchesHost(host, "ddinstagram.com")) return "instagram";
  if (matchesHost(host, "tiktok.com")) return "tiktok";
  if (matchesHost(host, "twitter.com") || matchesHost(host, "x.com") || matchesHost(host, "vxtwitter.com") || matchesHost(host, "fixvx.com")) return "twitter";
  if (matchesHost(host, "reddit.com") || host === "v.redd.it" || host === "redd.it") return "reddit";
  if (matchesHost(host, "twitch.tv")) return "twitch";
  if (host === "pin.it" || isPinterestHost(host)) return "pinterest";
  if (host === "bsky.app" || host.endsWith(".bsky.app")) return "bluesky";
  if (host === "t.me" || matchesHost(host, "telegram.me") || matchesHost(host, "telegram.org")) return "telegram";
  if (matchesHost(host, "vimeo.com")) return "vimeo";
  if (matchesHost(host, "udemy.com")) return "udemy";
  if (matchesHost(host, "bilibili.com") || host === "b23.tv") return "bilibili";

  return null;
}

function detectContentType(platform, url) {
  const segments = url.pathname.split("/").filter(Boolean);
  switch (platform) {
    case "youtube":
      return parseYouTube(url, segments);
    case "instagram":
      return parseInstagram(segments);
    case "tiktok":
      return parseTikTok(segments);
    case "twitter":
      return parseTwitter(segments);
    case "reddit":
      return parseReddit(url, segments);
    case "twitch":
      return parseTwitch(url, segments);
    case "hotmart":
      return parseHotmart(segments);
    case "pinterest":
      return parsePinterest(segments);
    case "bluesky":
      return parseBluesky(segments);
    case "telegram":
      return parseTelegram(segments);
    case "vimeo":
      return parseVimeo(segments);
    case "udemy":
      return parseUdemy(segments);
    case "bilibili":
      return parseBilibili(segments);
    default:
      return "unknown";
  }
}

function parseYouTube(url, segments) {
  const hasVideo = url.searchParams.get("v");
  if (hasVideo) {
    return url.searchParams.has("list") ? "playlist" : "video";
  }

  if (url.hostname.toLowerCase() === "youtu.be" && segments[0]) {
    return "video";
  }

  if (segments[0] === "shorts" && segments[1]) {
    return "short";
  }

  if (segments[0] === "playlist" && url.searchParams.get("list")) {
    return "playlist";
  }

  if ((segments[0] === "live" || segments[0] === "embed" || segments[0] === "v") && segments[1]) {
    return "video";
  }

  if (
    segments[0] === "channel" ||
    segments[0] === "c" ||
    (segments[0] && segments[0].startsWith("@"))
  ) {
    return "profile";
  }

  return "unknown";
}

function parseInstagram(segments) {
  switch (segments[0]) {
    case "p":
      return segments[1] ? "post" : "unknown";
    case "reel":
    case "reels":
      return segments[1] ? "reel" : "unknown";
    case "stories":
      return segments[2] ? "image" : "unknown";
    default:
      return segments[0] && !["explore", "accounts", "direct"].includes(segments[0]) ? "profile" : "unknown";
  }
}

function parseTikTok(segments) {
  if (segments[0]?.startsWith("@")) {
    if (segments[1] === "video" && segments[2]) {
      return "video";
    }
    return "profile";
  }

  // Short video links (/t/<code>) and embeds (/embed/v2/<id>)
  if (segments[0] === "t" && segments[1]) {
    return "video";
  }
  if (segments[0] === "embed" && segments[1] === "v2" && segments[2]) {
    return "video";
  }

  return "unknown";
}

function parseTwitter(segments) {
  if (segments.length >= 3 && segments[1] === "status" && segments[2]) {
    return "post";
  }

  return segments[0] && !["search", "explore", "settings", "i"].includes(segments[0]) ? "profile" : "unknown";
}

function parseReddit(url, segments) {
  const host = url.hostname.toLowerCase();

  if (host === "v.redd.it" || host === "redd.it") {
    return segments[0] ? "video" : "unknown";
  }

  if (segments.length >= 4 && segments[0] === "r" && segments[2] === "comments" && segments[3]) {
    return "post";
  }

  if (segments[0] === "comments" && segments[1]) {
    return "post";
  }

  if (segments[0] === "video" && segments[1]) {
    return "video";
  }

  if (segments[0] === "r") {
    if (segments.length >= 4 && segments[2] === "s" && segments[3]) {
      return "post";
    }
    return segments[1] ? "profile" : "unknown";
  }

  return "unknown";
}

function parseTwitch(url, segments) {
  const host = url.hostname.toLowerCase();

  if (segments[0] === "videos" && segments[1]) {
    return "video";
  }

  if (host.includes("clips.twitch.tv") && segments[0]) {
    return "clip";
  }

  if (segments.length >= 3 && segments[1] === "clip" && segments[2]) {
    return "clip";
  }

  const twitchNonChannel = ["directory", "settings", "downloads", "jobs", "turbo", "store", "inventory", "wallet", "subscriptions", "friends", "prime"];
  if (segments[0] && !twitchNonChannel.includes(segments[0])) {
    return "video"; // live stream — downloadable via yt-dlp
  }

  return "unknown";
}

function parseHotmart(segments) {
  return segments.some((segment) => ["club", "lesson", "course"].includes(segment)) ? "course" : "unknown";
}

function parsePinterest(segments) {
  if (segments[0] === "pin" && segments[1]) {
    return "image";
  }

  return "unknown";
}

function parseBluesky(segments) {
  if (segments.length >= 4 && segments[0] === "profile" && segments[2] === "post" && segments[3]) {
    return "post";
  }

  if (segments[0] === "profile" && segments[1]) {
    return "profile";
  }

  return "unknown";
}

function parseTelegram(segments) {
  if (segments.length >= 2 && /^\d+$/.test(segments[1])) {
    return "post";
  }

  return segments[0] && !["joinchat", "addstickers", "login", "share"].includes(segments[0]) ? "profile" : "unknown";
}

function parseVimeo(segments) {
  return segments[0] && /^\d+$/.test(segments[0]) ? "video" : "unknown";
}

function parseUdemy(segments) {
  return segments[0] === "course" && segments[1] ? "course" : "unknown";
}

function parseBilibili(segments) {
  return segments[0] === "video" && segments[1] ? "video" : "unknown";
}

function matchesHost(host, domain) {
  return host === domain || host.endsWith(`.${domain}`);
}

function isPinterestHost(host) {
  return /(?:^|\.)pinterest\.\w+(?:\.\w+)?$/.test(host);
}
