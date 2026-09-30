import { describe, expect, it } from "vitest";
import { defaultState, downloadArgs, linksIn, validSubs } from "./smart-presets";

describe("smart presets", () => {
  it("turns a preset into download_from_url arguments", () => {
    const [video, subs, audio] = defaultState().presets;
    expect(downloadArgs(video, "https://a.test/v", "/dl")).toMatchObject({
      outputDir: "/dl",
      downloadMode: null,
      quality: "1080p",
      subtitleLang: null,
    });
    expect(downloadArgs(subs, "https://a.test/v", "/dl").subtitleLang).toBe("en");
    expect(downloadArgs({ ...audio, subs: "en", folder: "/music" }, "https://a.test/v", "/dl")).toMatchObject({
      outputDir: "/music",
      downloadMode: "audio",
      quality: "best",
      subtitleLang: null,
    });
  });

  it("accepts language lists only", () => {
    expect(validSubs("")).toBe(true);
    expect(validSubs("en,pt-BR")).toBe(true);
    expect(validSubs("--exec x")).toBe(false);
    expect(validSubs("en ls")).toBe(false);
  });

  it("finds each link once in pasted text", () => {
    expect(linksIn("see https://a.test/1, and\nhttps://a.test/1 https://b.test/2).")).toEqual([
      "https://a.test/1",
      "https://b.test/2",
    ]);
    expect(linksIn("no links here")).toEqual([]);
  });
});
