import { describe, expect, it } from "vitest";
import { classifyError } from "./friendly-error";

describe("classifyError", () => {
  it.each([
    ["ERROR: [youtube] abc: Private video. Sign in if you've been granted access", "login"],
    ["ERROR: [youtube] x: Sign in to confirm your age. This video may be inappropriate for some users.", "age"],
    ["ERROR: [youtube] x: The uploader has not made this video available in your country", "geo"],
    ["ERROR: [youtube] x: This video is not available in your country", "geo"],
    ["ERROR: [youtube] x: Video unavailable. This video has been removed by the uploader", "removed"],
    ["ERROR: Unable to download webpage: HTTP Error 429: Too Many Requests", "rate"],
    ["ERROR: Unsupported URL: https://example.com/page", "unsupported"],
    ["ERROR: Postprocessing: ffprobe and ffmpeg not found", "ffmpeg"],
    ["ERROR: Unable to download webpage: <urlopen error [Errno 8] nodename nor servname provided> (caused by getaddrinfo failed)", "network"],
    ["ERROR: Read timed out.", "network"],
    ["something nobody planned for", "generic"],
  ])("%s → %s", (raw, kind) => {
    expect(classifyError(raw)).toBe(kind);
  });
});
