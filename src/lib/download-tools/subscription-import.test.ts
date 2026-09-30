import { describe, expect, it } from "vitest";
import { channelUrlFrom, parseSubscriptions } from "./subscription-import";

const A = "UCBR8-60-B28hp2BmDPdntcQ";
const B = "UCSMOQeBJ2RAnuFungnQOxLg";

describe("parseSubscriptions", () => {
  it("reads a FreeTube .db with one profile per line", () => {
    const db = [
      JSON.stringify({ name: "All Channels", _id: "allChannels", subscriptions: [{ id: A, name: "Channel A", thumbnail: "" }, { id: B, name: "Blender", thumbnail: null }] }),
      JSON.stringify({ name: "Art", _id: "x1", subscriptions: [{ id: B, name: "Blender", thumbnail: null }] }),
    ].join("\n");
    const r = parseSubscriptions(db);
    expect(r?.format).toBe("freetube");
    expect(r?.channels).toEqual([
      { url: `https://www.youtube.com/channel/${A}/videos`, title: "Channel A" },
      { url: `https://www.youtube.com/channel/${B}/videos`, title: "Blender" },
    ]);
  });

  it("reads the old FreeTube format with one channel per line", () => {
    const db = JSON.stringify({ channelId: A, channelName: "Channel A", channelThumbnail: "t", profile: [], _id: "1" });
    expect(parseSubscriptions(db)?.channels[0].url).toBe(`https://www.youtube.com/channel/${A}/videos`);
  });

  it("reads a NewPipe export and keeps other services' URLs", () => {
    const json = JSON.stringify({
      app_version: "0.27.0",
      subscriptions: [
        { service_id: 0, url: `https://www.youtube.com/channel/${A}`, name: "Channel A" },
        { service_id: 1, url: "https://soundcloud.com/someone", name: "Someone" },
      ],
    });
    const r = parseSubscriptions(json);
    expect(r?.format).toBe("newpipe");
    expect(r?.channels.map((c) => c.url)).toEqual([`https://www.youtube.com/channel/${A}/videos`, "https://soundcloud.com/someone"]);
  });

  it("reads the YouTube JSON export", () => {
    const json = JSON.stringify([{ snippet: { resourceId: { channelId: B }, title: "Blender" } }, { snippet: {} }]);
    const r = parseSubscriptions(json);
    expect(r?.format).toBe("youtube-json");
    expect(r?.channels).toHaveLength(1);
    expect(r?.skipped).toBe(1);
  });

  it("reads a Takeout CSV with quoted titles", () => {
    const csv = `Channel Id,Channel Url,Channel Title\r\n${A},http://www.youtube.com/channel/${A},"Hello, world"\r\n`;
    const r = parseSubscriptions(csv);
    expect(r?.format).toBe("csv");
    expect(r?.channels).toEqual([{ url: `https://www.youtube.com/channel/${A}/videos`, title: "Hello, world" }]);
  });

  it("reads OPML feeds, including Invidious ones", () => {
    const opml = `<?xml version="1.0"?><opml version="1.1"><body><outline text="YouTube Subscriptions" title="YouTube Subscriptions">
      <outline text="Tom &amp; Jerry" title="Tom &amp; Jerry" type="rss" xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id=${A}"/>
      <outline text="B" type="rss" xmlUrl="https://yewtu.be/feed/channel/${B}"/>
    </outline></body></opml>`;
    const r = parseSubscriptions(opml);
    expect(r?.format).toBe("opml");
    expect(r?.channels).toEqual([
      { url: `https://www.youtube.com/channel/${A}/videos`, title: "Tom & Jerry" },
      { url: `https://www.youtube.com/channel/${B}/videos`, title: "B" },
    ]);
  });

  it("reads a plain list and drops duplicates", () => {
    const r = parseSubscriptions(`# mine\nhttps://www.youtube.com/@blender\nhttps://youtube.com/@blender/\nnot a url\nhttps://vimeo.com/blender`);
    expect(r?.format).toBe("urls");
    expect(r?.channels.map((c) => c.url)).toEqual(["https://www.youtube.com/@blender/videos", "https://vimeo.com/blender"]);
    expect(r?.skipped).toBe(1);
  });

  it("returns null for text that is not a subscription list", () => {
    expect(parseSubscriptions("")).toBeNull();
    expect(parseSubscriptions("hello there")).toBeNull();
    expect(parseSubscriptions('{"foo": 1}')).toBeNull();
  });
});

describe("channelUrlFrom", () => {
  it("keeps playlists and rejects non-web schemes", () => {
    expect(channelUrlFrom("https://www.youtube.com/playlist?list=PLabc&si=x")).toBe("https://www.youtube.com/playlist?list=PLabc");
    expect(channelUrlFrom("javascript:alert(1)")).toBeNull();
  });
});
