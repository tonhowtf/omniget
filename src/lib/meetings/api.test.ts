import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { agentPrompt, classifyLink, clock, isBusy, speakerHue, turns } from "./api";

describe("meetings api helpers", () => {
  it("formats clocks", () => {
    expect(clock(0)).toBe("00:00");
    expect(clock(65_400)).toBe("01:05");
    expect(clock(3_725_000)).toBe("1:02:05");
  });

  it("merges turns of the same speaker", () => {
    const merged = turns([
      { start_ms: 0, end_ms: 1000, speaker: "A", text: "um" },
      { start_ms: 1500, end_ms: 2000, speaker: "A", text: "dois" },
      { start_ms: 2100, end_ms: 3000, speaker: "B", text: "três" },
      { start_ms: 9000, end_ms: 9500, speaker: "B", text: "quatro" },
    ]);
    expect(merged.map((s) => s.text)).toEqual(["um dois", "três", "quatro"]);
  });

  it("classifies pasted links", () => {
    expect(classifyLink("https://teams.microsoft.com/meet/3845612903427?p=Ab")).toBe("bot");
    expect(classifyLink("https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0")).toBe("bot");
    expect(classifyLink("https://meet.google.com/abc-defg-hij")).toBe("bot");
    expect(classifyLink("https://us02web.zoom.us/j/812?pwd=x")).toBe("bot");
    expect(classifyLink("https://contoso.sharepoint.com/:v:/s/Eng/Ea?e=1")).toBe("recording");
    expect(classifyLink("https://euc1.api.svc.ms/transform/videomanifest?part=index&format=dash")).toBe("recording");
    expect(classifyLink("https://www.youtube.com/watch?v=x")).toBeNull();
    expect(classifyLink("not a url")).toBeNull();
  });

  it("knows busy states", () => {
    expect(isBusy("transcribing")).toBe(true);
    expect(isBusy("ready")).toBe(false);
    expect(isBusy("bot_live")).toBe(false);
  });

  it("keeps a speaker's hue stable", () => {
    expect(speakerHue("Ana")).toBe(speakerHue("Ana"));
    expect(speakerHue(undefined)).toBe(220);
  });

  it("fences the minutes as data in the agent prompt", () => {
    const p = agentPrompt("# Sync\n", "Post to the Eng channel");
    expect(p.startsWith("Post to the Eng channel\n")).toBe(true);
    expect(p).toContain("<meeting>\n# Sync\n</meeting>");
  });
});

describe("calendar helpers", () => {
  it("flags meetings about to start or running", async () => {
    const { isSoon } = await import("./api");
    const now = Date.parse("2026-09-30T13:00:00Z");
    const m = (start: string, end: string) =>
      ({ id: "", subject: "", start, end, organizer: "", attendees: [], join_url: null, web_link: null, is_organizer: false });
    expect(isSoon(m("2026-09-30T13:10:00Z", "2026-09-30T13:40:00Z"), now)).toBe(true);
    expect(isSoon(m("2026-09-30T12:30:00Z", "2026-09-30T13:30:00Z"), now)).toBe(true);
    expect(isSoon(m("2026-09-30T14:00:00Z", "2026-09-30T15:00:00Z"), now)).toBe(false);
    expect(isSoon(m("2026-09-30T11:00:00Z", "2026-09-30T12:00:00Z"), now)).toBe(false);
  });
});
