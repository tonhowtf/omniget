import { describe, expect, it } from "vitest";
import { browserMayHelp, classifyError } from "./friendly-error";

describe("classifyError", () => {
  it("treats an Instagram login redirect as a login problem", () => {
    expect(classifyError("Instagram redirected to login; the post may not be public")).toBe("login");
  });
});

describe("browserMayHelp", () => {
  it("offers the embedded browser when the extractor fails on the site", () => {
    expect(browserMayHelp(classifyError("ERROR: Unable to extract video data; HTTP Error 403: Forbidden"))).toBe(true);
    expect(browserMayHelp("unsupported")).toBe(true);
    expect(browserMayHelp("login")).toBe(true);
  });
  it("does not offer it for problems the browser cannot fix", () => {
    expect(browserMayHelp("network")).toBe(false);
    expect(browserMayHelp("ffmpeg")).toBe(false);
    expect(browserMayHelp("removed")).toBe(false);
  });
});
