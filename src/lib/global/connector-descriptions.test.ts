import { describe, expect, it } from "vitest";
import { localizedConnectorDescription } from "./connector-descriptions";

describe("localizedConnectorDescription", () => {
  it("returns the Russian wording for a known helper", () => {
    expect(localizedConnectorDescription("udemy", "Courses you are enrolled in…", "ru")).toBe(
      "Курсы, на которые вы записаны: видео, субтитры, статьи, файлы, вложения, тесты и практические задания. Лекции с DRM показываются и пропускаются.",
    );
  });

  it("keeps the catalog text for a helper without a translation", () => {
    expect(localizedConnectorDescription("bunkr", "Albums and files.", "ru")).toBe("Albums and files.");
  });

  it("does not treat inherited object keys as translations", () => {
    expect(localizedConnectorDescription("constructor", "Catalog text.", "ru")).toBe("Catalog text.");
    expect(localizedConnectorDescription("toString", "Catalog text.", "ru")).toBe("Catalog text.");
  });

  it("returns the catalog text for other locales", () => {
    expect(localizedConnectorDescription("udemy", "Courses you are enrolled in…", "de")).toBe(
      "Courses you are enrolled in…",
    );
  });
});
