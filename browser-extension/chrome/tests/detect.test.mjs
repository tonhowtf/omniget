import test from "node:test";
import assert from "node:assert/strict";

import { detectSupportedMediaUrl } from "../src/detect.js";

// ── Helpers ──────────────────────────────────────────────────────────────

function assertSupported(url, platform, contentType) {
  const detected = detectSupportedMediaUrl(url);
  assert.equal(detected?.platform, platform, `platform for ${url}`);
  assert.equal(detected?.contentType, contentType, `contentType for ${url}`);
  assert.equal(detected?.supported, true, `expected supported for ${url}`);
}

function assertUnsupported(url, platform, contentType) {
  const detected = detectSupportedMediaUrl(url);
  assert.equal(detected?.platform, platform, `platform for ${url}`);
  assert.equal(detected?.contentType, contentType, `contentType for ${url}`);
  assert.equal(detected?.supported, false, `expected unsupported for ${url}`);
}

function assertNull(url) {
  assert.equal(detectSupportedMediaUrl(url), null, `expected null for ${url}`);
}

// ── Edge cases / invalid input ───────────────────────────────────────────

test("returns null for null/undefined/empty", () => {
  assertNull(null);
  assertNull(undefined);
  assertNull("");
});

test("returns null for non-http protocols", () => {
  assertNull("ftp://example.com/video.mp4");
  assertNull("file:///home/user/video.mp4");
  assertNull("magnet:?xt=urn:btih:abc");
});

test("returns null for malformed URLs", () => {
  assertNull("not a url at all");
  assertNull("http://");
});

test("returns null for unsupported sites", () => {
  assertNull("https://www.google.com/search?q=omniget");
  assertNull("https://github.com/tonhowtf/omniget");
  assertNull("https://example.com/video/123");
});

// ── YouTube ──────────────────────────────────────────────────────────────

test("YouTube: direct watch page", () => {
  assertSupported("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "youtube", "video");
});

test("YouTube: watch page with playlist param", () => {
  assertSupported("https://www.youtube.com/watch?v=abc&list=PLxyz", "youtube", "playlist");
});

test("YouTube: youtu.be short link", () => {
  assertSupported("https://youtu.be/dQw4w9WgXcQ", "youtube", "video");
});

test("YouTube: shorts", () => {
  assertSupported("https://www.youtube.com/shorts/abc123", "youtube", "short");
});

test("YouTube: playlist page", () => {
  assertSupported("https://www.youtube.com/playlist?list=PLxyz", "youtube", "playlist");
});

test("YouTube: live stream", () => {
  assertSupported("https://www.youtube.com/live/abc123", "youtube", "video");
});

test("YouTube: embed", () => {
  assertSupported("https://www.youtube.com/embed/abc123", "youtube", "video");
});

test("YouTube: old embed /v/ format", () => {
  assertSupported("https://www.youtube.com/v/abc123", "youtube", "video");
});

test("YouTube: channel page is unsupported", () => {
  assertUnsupported("https://www.youtube.com/@omniget", "youtube", "profile");
  assertUnsupported("https://www.youtube.com/channel/UCxyz", "youtube", "profile");
  assertUnsupported("https://www.youtube.com/c/omniget", "youtube", "profile");
});

test("YouTube: youtube-nocookie.com", () => {
  assertSupported("https://www.youtube-nocookie.com/embed/abc123", "youtube", "video");
});

// ── Instagram ────────────────────────────────────────────────────────────

test("Instagram: post", () => {
  assertSupported("https://www.instagram.com/p/CxyzABC/", "instagram", "post");
});

test("Instagram: reel", () => {
  assertSupported("https://www.instagram.com/reel/abc123/", "instagram", "reel");
});

test("Instagram: story", () => {
  assertSupported("https://www.instagram.com/stories/user/12345/", "instagram", "image");
});

test("Instagram: profile is unsupported", () => {
  assertUnsupported("https://www.instagram.com/username", "instagram", "profile");
});

test("Instagram: explore is unknown", () => {
  assertUnsupported("https://www.instagram.com/explore", "instagram", "unknown");
});

test("Instagram: ddinstagram.com mirror", () => {
  assertSupported("https://www.ddinstagram.com/reel/abc123/", "instagram", "reel");
});

// ── TikTok ───────────────────────────────────────────────────────────────

test("TikTok: user video", () => {
  assertSupported("https://www.tiktok.com/@user/video/1234567890", "tiktok", "video");
});

test("TikTok: profile is unsupported", () => {
  assertUnsupported("https://www.tiktok.com/@user", "tiktok", "profile");
});

test("TikTok: explore page is unknown", () => {
  assertUnsupported("https://www.tiktok.com/explore", "tiktok", "unknown");
});

test("TikTok: foryou page is unknown", () => {
  assertUnsupported("https://www.tiktok.com/foryou", "tiktok", "unknown");
});

// ── Twitter / X ──────────────────────────────────────────────────────────

test("Twitter: post/status", () => {
  assertSupported("https://twitter.com/user/status/123456", "twitter", "post");
  assertSupported("https://x.com/user/status/123456", "twitter", "post");
});

test("Twitter: vxtwitter mirror", () => {
  assertSupported("https://vxtwitter.com/user/status/123456", "twitter", "post");
});

test("Twitter: fixvx mirror", () => {
  assertSupported("https://fixvx.com/user/status/123456", "twitter", "post");
});

test("Twitter: profile is unsupported", () => {
  assertUnsupported("https://twitter.com/user", "twitter", "profile");
});

test("Twitter: search/explore/settings are unknown", () => {
  assertUnsupported("https://twitter.com/search", "twitter", "unknown");
  assertUnsupported("https://twitter.com/explore", "twitter", "unknown");
  assertUnsupported("https://twitter.com/settings", "twitter", "unknown");
  assertUnsupported("https://twitter.com/i", "twitter", "unknown");
});

// ── Reddit ───────────────────────────────────────────────────────────────

test("Reddit: subreddit post", () => {
  assertSupported("https://www.reddit.com/r/videos/comments/abc123/example/", "reddit", "post");
});

test("Reddit: short comment link", () => {
  assertSupported("https://www.reddit.com/comments/abc123/", "reddit", "post");
});

test("Reddit: v.redd.it direct link", () => {
  assertSupported("https://v.redd.it/abc123", "reddit", "video");
});

test("Reddit: redd.it short link", () => {
  assertSupported("https://redd.it/abc123", "reddit", "video");
});

test("Reddit: /r/sub/s/ share link", () => {
  assertSupported("https://www.reddit.com/r/funny/s/abc123/", "reddit", "post");
});

test("Reddit: /video/ link", () => {
  assertSupported("https://www.reddit.com/video/abc123", "reddit", "video");
});

test("Reddit: subreddit browse is unsupported profile", () => {
  assertUnsupported("https://www.reddit.com/r/videos", "reddit", "profile");
});

test("Reddit: homepage is unknown", () => {
  assertUnsupported("https://www.reddit.com/", "reddit", "unknown");
});

test("Reddit: random top-level path is unknown", () => {
  assertUnsupported("https://www.reddit.com/best", "reddit", "unknown");
});

// ── Twitch ───────────────────────────────────────────────────────────────

test("Twitch: VOD video", () => {
  assertSupported("https://www.twitch.tv/videos/123456", "twitch", "video");
});

test("Twitch: clip via clips subdomain", () => {
  assertSupported("https://clips.twitch.tv/FunnyClipName", "twitch", "clip");
});

test("Twitch: clip via /clip/ path", () => {
  assertSupported("https://www.twitch.tv/streamer/clip/ClipName", "twitch", "clip");
});

test("Twitch: live stream channel is supported as video", () => {
  assertSupported("https://www.twitch.tv/shroud", "twitch", "video");
});

test("Twitch: directory page is unknown", () => {
  assertUnsupported("https://www.twitch.tv/directory", "twitch", "unknown");
});

test("Twitch: settings page is unknown", () => {
  assertUnsupported("https://www.twitch.tv/settings", "twitch", "unknown");
});

// ── Hotmart ──────────────────────────────────────────────────────────────

test("Hotmart: course page", () => {
  assertSupported("https://app.hotmart.com/club/abc/lesson/xyz", "hotmart", "course");
});

test("Hotmart: generic page is unknown", () => {
  assertUnsupported("https://app.hotmart.com/dashboard", "hotmart", "unknown");
});

// ── Pinterest ────────────────────────────────────────────────────────────

test("Pinterest: pin page", () => {
  assertSupported("https://www.pinterest.com/pin/123456/", "pinterest", "image");
});

test("Pinterest: pin.it short link", () => {
  const detected = detectSupportedMediaUrl("https://pin.it/abc123");
  assert.equal(detected?.platform, "pinterest");
});

test("Pinterest: country TLD", () => {
  assertSupported("https://www.pinterest.co.uk/pin/123456/", "pinterest", "image");
  assertSupported("https://www.pinterest.fr/pin/123456/", "pinterest", "image");
  assertSupported("https://br.pinterest.com/pin/123456/", "pinterest", "image");
});

test("Pinterest: does not match notpinterest.com", () => {
  assertNull("https://notpinterest.com/pin/123");
});

test("Pinterest: profile page is unknown", () => {
  assertUnsupported("https://www.pinterest.com/username/", "pinterest", "unknown");
});

// ── Bluesky ──────────────────────────────────────────────────────────────

test("Bluesky: post", () => {
  assertSupported("https://bsky.app/profile/user.bsky.social/post/abc123", "bluesky", "post");
});

test("Bluesky: profile is unsupported", () => {
  assertUnsupported("https://bsky.app/profile/user.bsky.social", "bluesky", "profile");
});

// ── Telegram ─────────────────────────────────────────────────────────────

test("Telegram: numbered post", () => {
  assertSupported("https://t.me/channel/123", "telegram", "post");
});

test("Telegram: channel profile is unsupported", () => {
  assertUnsupported("https://t.me/channel", "telegram", "profile");
});

test("Telegram: joinchat is unknown", () => {
  assertUnsupported("https://t.me/joinchat", "telegram", "unknown");
});

test("Telegram: telegram.me domain", () => {
  assertSupported("https://telegram.me/channel/456", "telegram", "post");
});

// ── Vimeo ────────────────────────────────────────────────────────────────

test("Vimeo: numeric video ID", () => {
  assertSupported("https://vimeo.com/123456", "vimeo", "video");
});

test("Vimeo: non-numeric path is unknown", () => {
  assertUnsupported("https://vimeo.com/channels", "vimeo", "unknown");
});

// ── Udemy ────────────────────────────────────────────────────────────────

test("Udemy: course page", () => {
  assertSupported("https://www.udemy.com/course/my-course/", "udemy", "course");
});

test("Udemy: homepage is unknown", () => {
  assertUnsupported("https://www.udemy.com/", "udemy", "unknown");
});

// ── Bilibili ─────────────────────────────────────────────────────────────

test("Bilibili: video page", () => {
  assertSupported("https://www.bilibili.com/video/BV1xx411c7XY", "bilibili", "video");
});

test("Bilibili: b23.tv short link", () => {
  const detected = detectSupportedMediaUrl("https://b23.tv/abc123");
  assert.equal(detected?.platform, "bilibili");
});

test("Bilibili: homepage is unknown", () => {
  assertUnsupported("https://www.bilibili.com/", "bilibili", "unknown");
});
