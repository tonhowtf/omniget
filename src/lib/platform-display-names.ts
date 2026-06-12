export const PLATFORM_DISPLAY_NAMES: Record<string, string> = {
  bilibili: "Bilibili (哔哩哔哩)",
  douyin: "Douyin (抖音)",
  kuaishou: "Kuaishou (快手)",
  xiaohongshu: "Xiaohongshu (小红书)",
  tencentvideo: "Tencent Video (腾讯视频)",
  iqiyi: "iQiyi (爱奇艺)",
  mgtv: "Mango TV (芒果TV)",
  youku: "Youku (优酷)",
  youtube: "YouTube",
  instagram: "Instagram",
  tiktok: "TikTok",
  twitter: "Twitter / X",
  reddit: "Reddit",
  twitch: "Twitch",
  pinterest: "Pinterest",
  bluesky: "Bluesky",
  telegram: "Telegram",
  vimeo: "Vimeo",
  hotmart: "Hotmart",
  udemy: "Udemy",
  magnet: "BitTorrent",
  p2p: "P2P",
  direct_file: "Direct file",
};

export function platformDisplayName(s: string): string {
  return PLATFORM_DISPLAY_NAMES[s] || s.charAt(0).toUpperCase() + s.slice(1);
}
