import { describe, it, expect } from "vitest";
import { BUILD_INFO, formatBuildInfo } from "./build-info";

describe("BUILD_INFO", () => {
  it("exposes version from build-time define", () => {
    expect(BUILD_INFO.version).toBe("0.0.0-test");
  });

  it("derives a 7-char short commit hash", () => {
    expect(BUILD_INFO.commitShort).toBe("test-co");
    expect(BUILD_INFO.commitShort.length).toBe(7);
  });

  it("includes branch and date", () => {
    expect(BUILD_INFO.branch).toBe("test-branch");
    expect(BUILD_INFO.date).toBe("2026-04-13");
  });
});

describe("formatBuildInfo", () => {
  it("joins all non-empty build parts with ' · '", () => {
    const formatted = formatBuildInfo();
    expect(formatted).toContain("v0.0.0-test");
    expect(formatted).toContain("test-co");
    expect(formatted).toContain("test-branch");
    expect(formatted).toContain("2026-04-13");
    expect(formatted.split(" · ").length).toBe(4);
  });
});
