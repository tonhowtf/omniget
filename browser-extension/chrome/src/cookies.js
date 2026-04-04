const PLATFORM_COOKIE_DOMAINS = Object.freeze({
  youtube: [".youtube.com", ".google.com"],
  instagram: [".instagram.com", ".cdninstagram.com", ".fbcdn.net"],
  tiktok: [".tiktok.com", ".tiktokcdn.com"],
  twitter: [".twitter.com", ".x.com"],
  reddit: [".reddit.com"],
  twitch: [".twitch.tv", ".jtvnw.net"],
  vimeo: [".vimeo.com", ".vimeocdn.com"],
  bilibili: [".bilibili.com", ".bilivideo.com"],
  pinterest: [".pinterest.com"],
  hotmart: [".hotmart.com"],
  udemy: [".udemy.com"],
});

export async function extractCookiesForPlatform(platform, getAllCookies = defaultGetAllCookies) {
  const domains = PLATFORM_COOKIE_DOMAINS[platform];
  if (!domains) return null;

  const allCookies = [];
  for (const domain of domains) {
    const cookies = await getAllCookies({ domain });
    allCookies.push(...cookies);
  }

  if (allCookies.length === 0) return null;

  return allCookies.map((c) => ({
    domain: c.domain,
    httpOnly: c.httpOnly,
    path: c.path,
    secure: c.secure,
    expires: c.expirationDate ? Math.floor(c.expirationDate) : 0,
    name: c.name,
    value: c.value,
  }));
}

function defaultGetAllCookies(details) {
  if (typeof chrome === "undefined" || !chrome.cookies?.getAll) {
    return Promise.resolve([]);
  }
  return chrome.cookies.getAll(details);
}
