import { describe, expect, it } from "vitest";
import { cleanLink, linkKey, linksFromPaste } from "./paste-links";

describe("cleanLink", () => {
  it("drops tracking parameters and keeps the ones that matter", () => {
    expect(cleanLink("https://youtu.be/aqz-KE-bpKQ?si=abc123&t=42")).toBe("https://youtu.be/aqz-KE-bpKQ?t=42");
    expect(cleanLink("https://www.youtube.com/watch?v=aqz-KE-bpKQ&pp=ygU&feature=share")).toBe(
      "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
    );
    expect(cleanLink("https://example.com/video?id=7&utm_source=x&utm_medium=y&fbclid=z")).toBe(
      "https://example.com/video?id=7",
    );
    // `t` is a start time on YouTube and a tracker on X: only X loses it.
    expect(cleanLink("https://x.com/user/status/1?s=20&t=abc")).toBe("https://x.com/user/status/1");
    // `si` means something else on other sites and stays.
    expect(cleanLink("https://example.org/page?si=2")).toBe("https://example.org/page?si=2");
  });

  it("unwraps redirect pages", () => {
    const target = "https://video.blender.org/w/dnayqfMS4PAE9pSZ3kyVUb";
    expect(cleanLink(`https://www.youtube.com/redirect?event=video_description&q=${encodeURIComponent(target)}`)).toBe(target);
    expect(cleanLink(`https://l.facebook.com/l.php?u=${encodeURIComponent(target + "?fbclid=1")}&h=x`)).toBe(target);
    expect(cleanLink(`https://www.google.com/url?q=${encodeURIComponent(target)}&sa=D`)).toBe(target);
  });

  it("leaves anything that is not an http link alone", () => {
    expect(cleanLink("magnet:?xt=urn:btih:abc")).toBe("magnet:?xt=urn:btih:abc");
    expect(cleanLink("not a link")).toBe("not a link");
  });
});

describe("linkKey", () => {
  it("gives every shape of the same YouTube video one key", () => {
    const key = linkKey("https://www.youtube.com/watch?v=aqz-KE-bpKQ");
    expect(linkKey("https://youtu.be/aqz-KE-bpKQ")).toBe(key);
    expect(linkKey("https://m.youtube.com/watch?v=aqz-KE-bpKQ")).toBe(key);
    expect(linkKey("https://music.youtube.com/watch?v=aqz-KE-bpKQ")).toBe(key);
    expect(linkKey("https://www.youtube.com/shorts/aqz-KE-bpKQ")).toBe(key);
    expect(linkKey("https://youtu.be/aqz-KE-bpKQ?t=42")).not.toBe(key);
    expect(linkKey("https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PL1")).not.toBe(key);
  });
});

describe("linksFromPaste", () => {
  it("finds links in prose, lists and commas, cleaned and once each", () => {
    const text = [
      "Watch this: https://youtu.be/aqz-KE-bpKQ?si=zz, and (https://video.blender.org/w/abc).",
      "https://www.youtube.com/watch?v=aqz-KE-bpKQ&feature=share",
      "https://archive.org/details/The_Public_Domain_EP,https://example.com/a?utm_source=x",
    ].join("\n");
    expect(linksFromPaste(text)).toEqual([
      "https://youtu.be/aqz-KE-bpKQ",
      "https://video.blender.org/w/abc",
      "https://archive.org/details/The_Public_Domain_EP",
      "https://example.com/a",
    ]);
  });

  it("keeps magnets, torrent files and p2p codes as they are", () => {
    expect(linksFromPaste("magnet:?xt=urn:btih:abc film.torrent p2p:1234 hello")).toEqual([
      "magnet:?xt=urn:btih:abc",
      "film.torrent",
      "p2p:1234",
    ]);
  });
});
