import { describe, expect, it } from "vitest";
import { audioLanguages, pickedAudioLanguage } from "./audio-languages";

const format = (language: string | null | undefined, format_note: string | null, has_audio = true) => ({
  language,
  format_note,
  has_audio,
});

describe("audioLanguages", () => {
  it("lists each language once with the original first", () => {
    const list = audioLanguages([
      format("pa", "pa - dubbed"),
      format("pt", "Português - dubbed"),
      format("en-US", "American English - original (original)"),
      format("pt", "Portuguese, low"),
      format("en-US", "English (US) original (default), medium"),
    ]);
    expect(list).toEqual([
      { code: "en-US", original: true },
      { code: "pa", original: false },
      { code: "pt", original: false },
    ]);
  });

  it("ignores formats without audio, without a language or with a malformed code", () => {
    const list = audioLanguages([
      format("en", "1080p", false),
      format(null, "medium"),
      format(undefined, "low"),
      format("", "low"),
      format("pt]/b", "low"),
      format("fr", "French, low"),
    ]);
    expect(list).toEqual([{ code: "fr", original: false }]);
  });

  it("is empty for a video with no language information", () => {
    expect(audioLanguages([format(null, "medium"), format(null, "1080p", false)])).toEqual([]);
  });
});

describe("pickedAudioLanguage", () => {
  const list = [
    { code: "en-US", original: true },
    { code: "pt", original: false },
  ];

  it("sends nothing while the original track stays selected", () => {
    expect(pickedAudioLanguage(list, "en-US")).toBeNull();
    expect(pickedAudioLanguage(list, "")).toBeNull();
  });

  it("sends the chosen code for another track", () => {
    expect(pickedAudioLanguage(list, "pt")).toBe("pt");
  });

  it("never sends a code that is not in the list", () => {
    expect(pickedAudioLanguage(list, "de")).toBeNull();
  });
});
