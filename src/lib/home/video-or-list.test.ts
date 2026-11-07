import { describe, expect, it } from "vitest";
import { videoOrListLink } from "./video-or-list";

describe("videoOrListLink", () => {
  it("recognises an automatic mix link as a mix", () => {
    expect(videoOrListLink("https://www.youtube.com/watch?v=abc&list=RDabc&start_radio=1")).toEqual({
      videoId: "abc",
      listId: "RDabc",
      isMix: true,
    });
  });

  it("recognises a video opened inside a normal playlist", () => {
    expect(videoOrListLink("https://youtu.be/abc?list=PL1")?.isMix).toBe(false);
    expect(videoOrListLink("https://music.youtube.com/watch?v=abc&list=OLAK1")?.videoId).toBe("abc");
  });

  it("does not ask for a plain video or a plain playlist", () => {
    expect(videoOrListLink("https://www.youtube.com/watch?v=abc")).toBeNull();
    expect(videoOrListLink("https://www.youtube.com/playlist?list=PL1")).toBeNull();
    expect(videoOrListLink("https://example.com/watch?v=abc&list=PL1")).toBeNull();
    expect(videoOrListLink("not a link")).toBeNull();
  });
});
