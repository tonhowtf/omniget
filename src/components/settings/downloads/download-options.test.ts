import { describe, expect, it } from "vitest";
import { captionLocaleValue, customQuality, lowerQuality, qualityCard, ruleQualityOptions, saveMode } from "./download-options";
import { downloadsNav } from "$lib/settings-nav/downloads";

describe("download options", () => {
  it("maps the stored quality to a card or to the lower select", () => {
    expect(qualityCard("best")).toBe("best");
    expect(qualityCard("720p")).toBe("720p");
    expect(qualityCard("480p")).toBe("");
    expect(qualityCard("audio")).toBe("");
    expect(lowerQuality("360p")).toBe("360p");
    expect(lowerQuality("1080p")).toBe("");
    expect(lowerQuality(undefined)).toBe("");
  });

  it("reports a stored quality that no card or lower option shows", () => {
    expect(customQuality("1440p")).toBe("1440p");
    expect(customQuality("audio")).toBe("audio");
    expect(customQuality("best")).toBe("");
    expect(customQuality("480p")).toBe("");
    expect(customQuality("  ")).toBe("");
    expect(customQuality(undefined)).toBe("");
  });

  it("treats empty and auto subtitle language as the app language", () => {
    expect(captionLocaleValue("")).toBe("auto");
    expect(captionLocaleValue("AUTO")).toBe("auto");
    expect(captionLocaleValue(" pt ")).toBe("pt");
  });

  it("maps always ask to the save mode", () => {
    expect(saveMode(true)).toBe("ask");
    expect(saveMode(false)).toBe("folder");
  });

  it("keeps a custom rule quality visible in the select", () => {
    expect(ruleQualityOptions("")).toEqual(["best", "1080p", "720p", "480p", "360p"]);
    expect(ruleQualityOptions("1440p")).toContain("1440p");
    expect(ruleQualityOptions("720p")).toHaveLength(5);
  });
});

describe("downloads navigation", () => {
  it("points every alias and legacy link at a real section", () => {
    const sections = new Set(downloadsNav.sections);
    for (const alias of Object.values(downloadsNav.aliases)) {
      if ("href" in alias || alias.tab !== "downloads") continue;
      expect(sections.has(alias.section ?? "")).toBe(true);
    }
    for (const [tab, section] of Object.values(downloadsNav.legacy)) {
      if (tab === "downloads" && section) expect(sections.has(section)).toBe(true);
    }
    for (const section of downloadsNav.sections) expect(downloadsNav.sectionLabels[section]).toBeTruthy();
  });

  it("keeps the old section links working", () => {
    expect(downloadsNav.legacy["downloads|presets"]).toEqual(["downloads", "quality"]);
    expect(downloadsNav.legacy["downloads|clipboard"]).toEqual(["downloads", "shortcuts"]);
    expect(downloadsNav.legacy["downloads|hotkeys"]).toEqual(["downloads", "shortcuts"]);
    expect(downloadsNav.legacy["downloads|output"]).toEqual(["downloads", "files"]);
    expect(downloadsNav.legacy["downloads|templates"]).toEqual(["downloads", "files"]);
    expect(downloadsNav.legacy["cookies|"]).toEqual(["downloads", "cookies"]);
    expect(downloadsNav.legacy["channels|"]).toEqual(["downloads", "channels"]);
    expect(downloadsNav.aliases["settings.download.section_output"]).toEqual({ tab: "downloads", section: "files" });
  });
});
