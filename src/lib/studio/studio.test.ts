import { describe, expect, it } from "vitest";
import { assColor, assTime, buildAss, escapeAssText, karaokeText } from "./ass";
import { blurRadius, cropRect, even, hasCropSlack, introLayout, introLines, outputSize, wrapText } from "./frame";
import { activeCue, balance, greedyLines, layoutCues } from "./layout";
import { buildRequest, nothingToChange, stemOf } from "./plan";
import { applyPreset, geometry } from "./presets";
import { radioStep, rovingIndex } from "./radio";
import { milestone, pendingDownload, pendingTranscribe, trackDownload, trackTranscribe } from "./session";
import {
  byStart,
  editText,
  estimateWords,
  formatClock,
  formatTimestamp,
  fromRaw,
  insertAfter,
  makeSegment,
  mergeWithNext,
  parseSubtitles,
  parseTimestamp,
  retime,
  setTiming,
  shiftAll,
  shiftSegment,
  splitAt,
  toSrt,
  toVtt,
  wordIndexAt,
} from "./subtitles";
import type { AsrStatus, IntroSettings, Probe, Transcript, Word } from "./types";

const w = (text: string, start_ms: number, end_ms: number): Word => ({ text, start_ms, end_ms });

describe("timestamps", () => {
  it("formats SRT and VTT times", () => {
    expect(formatTimestamp(3_723_045, ",")).toBe("01:02:03,045");
    expect(formatTimestamp(-5, ".")).toBe("00:00:00.000");
    expect(formatClock(65_432)).toBe("1:05.4");
  });

  it("parses loose timestamps", () => {
    expect(parseTimestamp("01:02:03,045")).toBe(3_723_045);
    expect(parseTimestamp("02:03.5")).toBe(123_500);
    expect(parseTimestamp("0:00:01")).toBe(1000);
    expect(parseTimestamp("nope")).toBeNull();
  });
});

describe("words", () => {
  it("estimates word timing by length and keeps the span", () => {
    const words = estimateWords("a bbbb", 0, 700);
    expect(words).toEqual([w("a", 0, 200), w("bbbb", 200, 700)]);
    expect(estimateWords("   ", 0, 100)).toEqual([]);
  });

  it("keeps real word timings when they match the text", () => {
    const real = [w("hi", 0, 100), w("there", 300, 900)];
    const seg = makeSegment(0, 900, "hi  there", real);
    expect(seg.estimated).toBe(false);
    expect(seg.words).toBe(real);
    expect(makeSegment(0, 900, "hello there", real).estimated).toBe(true);
  });
});

describe("transcript edits", () => {
  const list = fromRaw([
    { start_ms: 2000, end_ms: 4000, text: "third line here" },
    { start_ms: 0, end_ms: 1000, text: "first one" },
    { start_ms: 1000, end_ms: 2000, text: " second " },
    { start_ms: 5000, end_ms: 6000, text: "   " },
  ]);

  it("sorts and drops empty lines", () => {
    expect(list.map((s) => s.text)).toEqual(["first one", "second", "third line here"]);
  });

  it("edits text and re-estimates words", () => {
    const e = editText(list[0], "only");
    expect(e.id).toBe(list[0].id);
    expect(e.words).toEqual([w("only", 0, 1000)]);
  });

  it("merges a line with the next", () => {
    const m = mergeWithNext(list, 0);
    expect(m).toHaveLength(2);
    expect(m[0].text).toBe("first one second");
    expect([m[0].start_ms, m[0].end_ms]).toEqual([0, 2000]);
    expect(m[0].words).toHaveLength(3);
    expect(mergeWithNext(list, 2)).toBe(list);
  });

  it("splits at the caret or in the middle", () => {
    expect(wordIndexAt("third line here", 6)).toBe(1);
    expect(wordIndexAt("third line here", 7)).toBe(1);
    expect(wordIndexAt("third line here", 10)).toBe(2);
    const s = splitAt(list, 2, 6);
    expect(s).toHaveLength(4);
    expect(s[2].text).toBe("third");
    expect(s[3].text).toBe("line here");
    expect(s[2].end_ms).toBe(s[3].start_ms);
    expect(s[3].end_ms).toBe(4000);
    const mid = splitAt(list, 2);
    expect(mid[2].text).toBe("third line");
    expect(splitAt(list, 1)).toBe(list);
  });

  it("shifts without going below zero", () => {
    expect(shiftSegment(list[1], -5000).start_ms).toBe(0);
    const moved = shiftAll(list, -500);
    expect(moved[0].start_ms).toBe(0);
    expect(moved[2].start_ms).toBe(2000);
    expect(shiftAll(list, 250)[0].words[0].start_ms).toBe(250);
  });

  it("retimes and rescales words", () => {
    const r = setTiming(list[0], 1000, 3000);
    expect([r.start_ms, r.end_ms]).toEqual([1000, 3000]);
    expect(r.words[r.words.length - 1].end_ms).toBe(3000);
    expect(setTiming(list[0], 500, 100).end_ms).toBe(700);
  });

  it("inserts an empty line after another", () => {
    const n = insertAfter(list, 0, 10_000);
    expect(n).toHaveLength(4);
    expect([n[1].start_ms, n[1].end_ms]).toEqual([1000, 1000 + 200]);
  });
});

describe("SRT and VTT", () => {
  const list = fromRaw([
    { start_ms: 0, end_ms: 1500, text: "Hello world" },
    { start_ms: 1500, end_ms: 3000, text: "Second" },
  ]);

  it("writes SRT", () => {
    expect(toSrt(list)).toBe("1\n00:00:00,000 --> 00:00:01,500\nHello world\n\n2\n00:00:01,500 --> 00:00:03,000\nSecond\n");
  });

  it("writes VTT", () => {
    expect(toVtt(list)).toBe("WEBVTT\n\n00:00:00.000 --> 00:00:01.500\nHello world\n\n00:00:01.500 --> 00:00:03.000\nSecond\n");
  });

  it("round-trips both formats", () => {
    for (const text of [toSrt(list), toVtt(list)]) {
      const back = parseSubtitles(text);
      expect(back.map((s) => [s.start_ms, s.end_ms, s.text])).toEqual([
        [0, 1500, "Hello world"],
        [1500, 3000, "Second"],
      ]);
    }
  });

  it("parses messy files with markup, BOM and cue settings", () => {
    const vtt = "﻿WEBVTT\r\n\r\nNOTE hi\r\n\r\nid-1\r\n00:01.000 --> 00:02.000 align:start\r\n<b>Bold</b> &amp; {\\an8}tags\r\nsecond line\r\n\r\nbroken --> nope\r\nx\r\n";
    const out = parseSubtitles(vtt);
    expect(out).toHaveLength(1);
    expect(out[0].text).toBe("Bold & tags second line");
    expect(out[0].start_ms).toBe(1000);
  });
});

describe("line layout", () => {
  const words = "one two three four five six seven".split(" ").map((t, i) => w(t, i * 100, i * 100 + 100));

  it("breaks greedily by characters", () => {
    expect(greedyLines(words, 9).map((l) => l.map((x) => x.text).join(" "))).toEqual(["one two", "three", "four five", "six seven"]);
    expect(greedyLines([w("supercalifragilistic", 0, 1)], 5)).toHaveLength(1);
  });

  it("balances two lines", () => {
    const b = balance(words.slice(0, 4), 20);
    expect(b.map((l) => l.map((x) => x.text).join(" "))).toEqual(["one two", "three four"]);
  });

  it("chunks a long segment into timed cues", () => {
    const seg = { ...makeSegment(0, 700, words.map((x) => x.text).join(" "), words) };
    const cues = layoutCues([seg], { maxChars: 9, maxLines: 2, uppercase: true });
    expect(cues).toHaveLength(2);
    expect(cues[0].lines.flat().map((x) => x.text)).toEqual(["ONE", "TWO", "THREE"]);
    expect([cues[0].start_ms, cues[0].end_ms]).toEqual([0, 300]);
    expect([cues[1].start_ms, cues[1].end_ms]).toEqual([300, 700]);
    expect(activeCue(cues, 350)).toBe(cues[1]);
    expect(activeCue(cues, 700)).toBeNull();
  });
});

describe("ASS", () => {
  it("converts colors and times", () => {
    expect(assColor("#FF9F0A")).toBe("&H000A9FFF");
    expect(assColor("#000000", 128)).toBe("&H80000000");
    expect(assColor("bad")).toBe("&H00FFFFFF");
    expect(assTime(3_723_456)).toBe("1:02:03.46");
  });

  it("escapes override characters", () => {
    expect(escapeAssText("a{b}\\N")).toBe("a(b)⧵N");
  });

  it("writes karaoke tags that add up to the cue", () => {
    const cue = { start_ms: 0, end_ms: 1000, lines: [[w("Hi", 100, 400), w("you", 400, 700)], [w("there", 700, 1000)]] };
    const text = karaokeText(cue);
    expect(text).toBe("{\\k10}{\\k30}Hi {\\k30}you\\N{\\k30}there");
    const sum = [...text.matchAll(/\\k(\d+)/g)].reduce((a, m) => a + Number(m[1]), 0);
    expect(sum).toBe(100);
  });

  it("builds a full document scaled to the output", () => {
    const style = applyPreset("bold_pop");
    const cues = layoutCues([makeSegment(0, 2000, "hello big world")], { maxChars: 18, maxLines: 2, uppercase: true });
    const ass = buildAss(cues, style, 1080, 1920);
    expect(ass).toContain("PlayResX: 1080\nPlayResY: 1920");
    expect(ass).toContain("Style: Default,Liberation Sans,84,&H000AD6FF,&H00FFFFFF,&H00000000,&H80000000,-1,");
    expect(ass).toContain(",1,6,0,5,65,65,154,1");
    expect(ass).toMatch(/Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,\{\\k\d+\}HELLO /);
    const boxed = buildAss(cues, applyPreset("boxed"), 1920, 1080);
    expect(boxed).toContain(",3,12,0,2,");
  });

  it("places captions by position and alignment", () => {
    const s = applyPreset("clean");
    expect(geometry({ ...s, position: "top" }, 1920, 1080).alignment).toBe(8);
    expect(geometry({ ...s, position: "middle", align: "left" }, 1920, 1080).alignment).toBe(4);
    expect(geometry(applyPreset("minimal"), 1920, 1080)).toMatchObject({ alignment: 1, marginV: 130 });
  });
});

describe("frame math", () => {
  it("matches the Rust crop and output sizes", () => {
    expect(even(607.5)).toBe(606);
    expect(cropRect(1920, 1080, 9, 16, 0.5)).toEqual({ w: 606, h: 1080, x: 657, y: 0 });
    expect(cropRect(1920, 1080, 9, 16, 1).x).toBe(1314);
    expect(cropRect(1080, 1920, 1, 1, 0.9)).toEqual({ w: 1080, h: 1080, x: 0, y: 420 });
    expect(outputSize(1921, 1081, "keep", "fit")).toEqual([1920, 1080]);
    expect(outputSize(1920, 1080, "9:16", "fit")).toEqual([1080, 1920]);
    expect(outputSize(1920, 1080, "4:5", "fit")).toEqual([1080, 1350]);
    expect(outputSize(3840, 4000, "1:1", "fit")).toEqual([2160, 2160]);
    expect(hasCropSlack(1920, 1080, "9:16")).toBe(true);
    expect(hasCropSlack(1080, 1920, "9:16")).toBe(false);
    expect(blurRadius(1080, 1920)).toBe(27);
  });

  it("matches the Rust intro layout", () => {
    const l = introLayout(1080, 1920, 2, 1);
    expect(l.titleSize).toBe(92);
    expect(l.subtitleSize).toBe(49);
    expect(l.titleYs[0]).toBeLessThan(l.titleYs[1]);
  });

  it("wraps and truncates intro text", () => {
    expect(wrapText("one two three four", 9, 3)).toEqual(["one two", "three", "four"]);
    expect(wrapText("one two three four", 9, 2)).toEqual(["one two", "three fo…"]);
    expect(wrapText("  ", 9, 2)).toEqual([]);
    expect(introLines(1080, 1920, "My video", "").subtitle).toEqual([]);
  });
});

describe("render request", () => {
  const probe: Probe = { path: "/v/clip.mov", name: "clip.mov", size_bytes: 1, duration_ms: 5000, width: 1920, height: 1080, fps: 30, has_video: true, has_audio: true, video_codec: "h264" };
  const intro: IntroSettings = { enabled: false, seconds: 3, title: "", subtitle: "", background: "#111111", color: "#FFFFFF", fade: true };
  const base = {
    probe,
    segments: fromRaw([{ start_ms: 0, end_ms: 1000, text: "hi" }]),
    burnCaptions: true,
    style: applyPreset("clean"),
    reframe: { aspect: "9:16" as const, mode: "crop" as const, position: 0.5 },
    intro,
    fadeOut: false,
    quality: "balanced" as const,
    outDir: null,
  };

  it("builds captions for the reframed size", () => {
    const req = buildRequest(base);
    expect(req.ass).toContain("PlayResX: 606\nPlayResY: 1080");
    expect(req.intro).toBeNull();
    expect(nothingToChange(base)).toBe(false);
  });

  it("drops empty extras", () => {
    const plain = { ...base, burnCaptions: false, reframe: { aspect: "keep" as const, mode: "crop" as const, position: 0.5 }, intro: { ...intro, enabled: true } };
    expect(buildRequest(plain).intro).toBeNull();
    expect(nothingToChange(plain)).toBe(true);
    const withIntro = buildRequest({ ...plain, intro: { ...intro, enabled: true, title: "Hello", seconds: 40 } });
    expect(withIntro.intro).toMatchObject({ seconds: 15, title_lines: ["Hello"], subtitle_lines: [] });
  });

  it("names exports after the video", () => {
    expect(stemOf("my.clip.mov")).toBe("my.clip");
    expect(stemOf(".mov")).toBe("captions");
  });
});

describe("timing edits keep the list in time order", () => {
  it("moves a line past its neighbour and keeps ids", () => {
    const list = [makeSegment(0, 1000, "one"), makeSegment(1000, 2000, "two"), makeSegment(2000, 3000, "three")];
    const moved = retime(list, 0, setTiming(list[0], 2500, 2900));
    expect(moved.map((s) => s.text)).toEqual(["two", "three", "one"]);
    expect(moved[2].id).toBe(list[0].id);
    expect(toSrt(moved).split("\n\n")[0]).toContain("two");
  });

  it("keeps the original order for equal starts", () => {
    const list = [makeSegment(500, 900, "a"), makeSegment(500, 800, "b")];
    expect(byStart(list).map((s) => s.text)).toEqual(["a", "b"]);
  });
});

describe("radio groups", () => {
  it("wraps with arrows and jumps with Home and End", () => {
    expect(radioStep("ArrowRight", 2, 3)).toBe(0);
    expect(radioStep("ArrowLeft", 0, 3)).toBe(2);
    expect(radioStep("ArrowDown", 0, 3)).toBe(1);
    expect(radioStep("ArrowUp", 1, 3)).toBe(0);
    expect(radioStep("Home", 2, 3)).toBe(0);
    expect(radioStep("End", 0, 3)).toBe(2);
    expect(radioStep("a", 0, 3)).toBeNull();
    expect(radioStep("ArrowRight", -1, 3)).toBe(1);
  });

  it("keeps one stop in the tab order", () => {
    expect(rovingIndex(["a", "b"], "b")).toBe(1);
    expect(rovingIndex(["a", "b"], "z")).toBe(0);
  });
});

describe("session", () => {
  it("announces progress only at milestones", () => {
    expect([0, 12, 25, 49, 50, 99, 100, NaN].map((p) => milestone(p))).toEqual([0, 0, 25, 25, 50, 75, 100, 0]);
  });

  it("remembers a running job until it settles", async () => {
    let finish!: (t: Transcript) => void;
    const promise = new Promise<Transcript>((r) => (finish = r));
    const entry = trackTranscribe("j1", "/a.mp4", promise);
    expect(pendingTranscribe()).toBe(entry);
    finish({ lang: "en", model: "m", segments: [] });
    await promise;
    await Promise.resolve();
    expect(pendingTranscribe()).toBeNull();

    const failing = Promise.reject(new Error("x")) as Promise<AsrStatus>;
    trackDownload("whisper-base", failing);
    expect(pendingDownload()?.model).toBe("whisper-base");
    await failing.catch(() => {});
    await Promise.resolve();
    expect(pendingDownload()).toBeNull();
  });
});
