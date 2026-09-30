import { describe, expect, it } from "vitest";
import { bestAvatar, fmtBytes, fmtCount, fmtDuration, thumbUrl, timeAgo, videoThumb } from "./format";

describe("fmtDuration", () => {
  it("formats minutes and hours like YouTube", () => {
    expect(fmtDuration(0)).toBe("0:00");
    expect(fmtDuration(5)).toBe("0:05");
    expect(fmtDuration(75)).toBe("1:15");
    expect(fmtDuration(600)).toBe("10:00");
    expect(fmtDuration(3723)).toBe("1:02:03");
    expect(fmtDuration(59.9)).toBe("0:59");
  });
  it("handles junk", () => {
    expect(fmtDuration(null)).toBe("0:00");
    expect(fmtDuration(undefined)).toBe("0:00");
    expect(fmtDuration(NaN)).toBe("0:00");
    expect(fmtDuration(-4)).toBe("0:00");
  });
});

describe("fmtCount", () => {
  it("compacts and truncates instead of rounding", () => {
    expect(fmtCount(999, "en")).toBe("999");
    expect(fmtCount(1299, "en")).toBe("1.2K");
    expect(fmtCount(1_290_000, "en")).toBe("1.2M");
    expect(fmtCount(1_999_999, "en")).toBe("1.9M");
    expect(fmtCount(3_450_000_000, "en")).toBe("3.4B");
  });
  it("returns empty for missing numbers", () => {
    expect(fmtCount(null)).toBe("");
  });
});

describe("thumbUrl", () => {
  const thumbs = [
    { url: "//i.ytimg.com/a.jpg", width: 120, height: 90 },
    { url: "https://i.ytimg.com/b.jpg", width: 320, height: 180 },
    { url: "http://i.ytimg.com/c.jpg", width: 1280, height: 720 },
  ];
  it("picks the smallest that covers the width at the pixel ratio", () => {
    expect(thumbUrl(thumbs, 100, 1)).toBe("https://i.ytimg.com/a.jpg");
    expect(thumbUrl(thumbs, 300, 1)).toBe("https://i.ytimg.com/b.jpg");
    expect(thumbUrl(thumbs, 300, 2)).toBe("https://i.ytimg.com/c.jpg");
  });
  it("falls back to the largest and forces https", () => {
    expect(thumbUrl(thumbs, 4000, 1)).toBe("https://i.ytimg.com/c.jpg");
    expect(thumbUrl([], 100)).toBe("");
    expect(thumbUrl(null, 100)).toBe("");
  });
  it("builds id thumbnails", () => {
    expect(videoThumb("abc")).toBe("https://i.ytimg.com/vi/abc/mqdefault.jpg");
  });
});

describe("bestAvatar", () => {
  it("rewrites the size of ggpht avatars", () => {
    expect(bestAvatar("https://yt3.ggpht.com/xyz=s88-c-k-c0x00ffffff-no-rj", 36, 2)).toBe("https://yt3.ggpht.com/xyz=s72-c-k-c0x00ffffff-no-rj");
    expect(bestAvatar("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj", 60, 1)).toBe("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj");
  });
  it("accepts thumbs and leaves other hosts alone", () => {
    expect(bestAvatar([{ url: "https://example.com/a.png", width: 88, height: 88 }], 40, 1)).toBe("https://example.com/a.png");
    expect(bestAvatar(null)).toBe("");
  });
});

describe("timeAgo", () => {
  const now = Date.UTC(2026, 8, 30);
  it("uses the largest unit", () => {
    expect(timeAgo(now - 3 * 86400e3, now, "en")).toBe("3 days ago");
    expect(timeAgo(now - 2 * 3600e3, now, "en")).toBe("2 hours ago");
    expect(timeAgo(now - 400 * 86400e3, now, "en")).toBe("1 year ago");
    expect(timeAgo(now - 5e3, now, "en")).toBe("5 seconds ago");
  });
  it("is empty without a date", () => {
    expect(timeAgo(null, now)).toBe("");
  });
});

describe("fmtBytes", () => {
  it("formats binary units", () => {
    expect(fmtBytes(512)).toBe("512 B");
    expect(fmtBytes(1536)).toBe("1.5 KB");
    expect(fmtBytes(20 * 1024 * 1024)).toBe("20 MB");
  });
});
