import { describe, expect, it } from "vitest";
import {
  absUrl,
  audioTracks,
  chooseQuality,
  cuesAt,
  decideAudioSync,
  isWebKit,
  loudnessGain,
  manifestUrl,
  mediaUrl,
  parseVtt,
  pickAudio,
  pickDirectVideo,
  qualityLabel,
  sbForget,
  sbStep,
  storyboardFrame,
  trimRate,
  videoLadder,
} from "./dash";
import type { Format, SbSegment } from "./types";

function f(p: Partial<Format> & Pick<Format, "itag" | "kind" | "codecs">): Format {
  return {
    mime: p.kind === "audio" ? (p.codecs.startsWith("opus") ? "audio/webm" : "audio/mp4") : p.codecs.startsWith("vp9") ? "video/webm" : "video/mp4",
    bitrate: 1000,
    audio_default: false,
    auto_dubbed: false,
    hdr: false,
    drc: false,
    init_range: { start: 0, end: 700 },
    index_range: { start: 701, end: 2000 },
    ...p,
  };
}

const FORMATS: Format[] = [
  f({ itag: 137, kind: "video", codecs: "avc1.640028", height: 1080, width: 1920, fps: 30, bitrate: 4_000_000 }),
  f({ itag: 136, kind: "video", codecs: "avc1.4d401f", height: 720, width: 1280, fps: 30, bitrate: 2_000_000 }),
  f({ itag: 298, kind: "video", codecs: "avc1.4d4020", height: 720, width: 1280, fps: 60, bitrate: 3_000_000 }),
  f({ itag: 134, kind: "video", codecs: "avc1.4d401e", height: 360, width: 640, fps: 30, bitrate: 600_000 }),
  f({ itag: 313, kind: "video", codecs: "vp9", height: 2160, width: 3840, fps: 30, bitrate: 15_000_000 }),
  f({ itag: 248, kind: "video", codecs: "vp9", height: 1080, width: 1920, fps: 30, bitrate: 3_000_000 }),
  f({ itag: 244, kind: "video", codecs: "vp9", height: 480, width: 854, fps: 30, bitrate: 800_000, init_range: null }),
  f({ itag: 140, kind: "audio", codecs: "mp4a.40.2", bitrate: 130_000, audio_default: true, audio_lang: "en" }),
  f({ itag: 251, kind: "audio", codecs: "opus", bitrate: 150_000, audio_default: true, audio_lang: "en" }),
  f({ itag: 250, kind: "audio", codecs: "opus", bitrate: 70_000, audio_default: true, audio_lang: "en" }),
  f({ itag: 1251, kind: "audio", codecs: "opus", bitrate: 150_000, audio_lang: "pt", auto_dubbed: true, audio_name: "Portuguese" }),
  f({ itag: 18, kind: "muxed", codecs: "avc1.42001E, mp4a.40.2", height: 360, bitrate: 500_000 }),
];

describe("videoLadder", () => {
  it("keeps one codec family, one rung per height, tallest first", () => {
    const l = videoLadder(FORMATS, { codec: "auto", webkit: true });
    expect(l.map((x) => x.itag)).toEqual([137, 298, 134]);
  });
  it("prefers VP9 off WebKit and honours the codec setting", () => {
    expect(videoLadder(FORMATS, { codec: "auto", webkit: false }).map((x) => x.itag)).toEqual([313, 248, 244]);
    expect(videoLadder(FORMATS, { codec: "avc1", webkit: false })[0].itag).toBe(137);
  });
  it("drops formats dash.js can't index when asked", () => {
    expect(videoLadder(FORMATS, { codec: "vp9", webkit: false, needRanges: true }).map((x) => x.itag)).toEqual([313, 248]);
  });
  it("skips a family the webview can't decode", () => {
    const l = videoLadder(FORMATS, { codec: "auto", webkit: false, supports: (x) => !x.codecs.startsWith("vp9") });
    expect(l[0].itag).toBe(137);
  });
  it("labels", () => {
    expect(qualityLabel({ height: 720, fps: 60, hdr: false })).toBe("720p60");
    expect(qualityLabel({ height: 2160, fps: 30, hdr: true })).toBe("2160p HDR");
  });
});

describe("chooseQuality", () => {
  const ladder = videoLadder(FORMATS, { codec: "vp9", webkit: false });
  it("respects an explicit ceiling", () => {
    expect(chooseQuality(ladder, "1080", 400)?.itag).toBe(248);
    expect(chooseQuality(ladder, 720, 400)?.itag).toBe(244);
    expect(chooseQuality(ladder, 144, 400)?.itag).toBe(244);
  });
  it("auto fits the box and caps at 1080p", () => {
    expect(chooseQuality(ladder, "auto", 300, 1)?.itag).toBe(244);
    expect(chooseQuality(ladder, "auto", 700, 1)?.itag).toBe(248);
    expect(chooseQuality(ladder, "auto", 1400, 2)?.itag).toBe(248);
  });
  it("is null on an empty ladder", () => {
    expect(chooseQuality([], "auto", 500)).toBeNull();
  });
});

describe("pickAudio", () => {
  it("prefers AAC on WebKit and Opus elsewhere, original track", () => {
    expect(pickAudio(FORMATS, { quality: "high", webkit: true })?.itag).toBe(140);
    expect(pickAudio(FORMATS, { quality: "high", webkit: false })?.itag).toBe(251);
    expect(pickAudio(FORMATS, { quality: "low", webkit: false })?.itag).toBe(250);
  });
  it("switches to a dub track", () => {
    const tracks = audioTracks(FORMATS);
    expect(tracks[0].original).toBe(true);
    const pt = tracks.find((t) => t.lang === "pt")!;
    expect(pickAudio(FORMATS, { quality: "high", webkit: false, track: pt.key })?.itag).toBe(1251);
  });
  it("direct video picks under the height", () => {
    expect(pickDirectVideo(FORMATS, { codec: "auto", webkit: true }, 720)?.itag).toBe(298);
    expect(pickDirectVideo(FORMATS, { codec: "auto", webkit: true }, 100)?.itag).toBe(134);
  });
});

describe("proxy urls", () => {
  // the backend sends the session root as `base`
  const set = { base: "http://127.0.0.1:5000/yt/s1", token: "t k", session: "s1", hls: null };
  it("builds media and manifest urls with the token", () => {
    expect(mediaUrl(set, 140)).toBe("http://127.0.0.1:5000/yt/s1/140?k=t%20k");
    expect(manifestUrl(set, [137, 136], [140])).toBe("http://127.0.0.1:5000/yt/s1/manifest.mpd?v=137,136&a=140&k=t%20k");
    expect(absUrl(set, "/yt/s1/cc/0.vtt")).toBe("http://127.0.0.1:5000/yt/s1/cc/0.vtt?k=t%20k");
    expect(absUrl(set, "http://127.0.0.1:5000/yt/s1/cc/0.vtt?k=abc")).toBe("http://127.0.0.1:5000/yt/s1/cc/0.vtt?k=abc");
    expect(absUrl(set, "https://i.ytimg.com/x.jpg")).toBe("https://i.ytimg.com/x.jpg");
  });
});

describe("decideAudioSync", () => {
  const base = { held: false, videoAdvancing: true, audioSeeking: false, audioReadyState: 4, msSinceLastRealign: 5000 };
  it("stays steady inside the tolerance", () => {
    expect(decideAudioSync({ ...base, drift: 0.3 })).toBe("steady");
    expect(decideAudioSync({ ...base, drift: -0.49 })).toBe("steady");
  });
  it("realigns real drift, but not too often", () => {
    expect(decideAudioSync({ ...base, drift: -1 })).toBe("realign");
    expect(decideAudioSync({ ...base, drift: 1 })).toBe("realign");
    expect(decideAudioSync({ ...base, drift: 1, msSinceLastRealign: 200 })).toBe("wait");
    expect(decideAudioSync({ ...base, drift: 1, audioReadyState: 1 })).toBe("wait");
  });
  it("holds audio ahead of a stalled video, then resumes", () => {
    expect(decideAudioSync({ ...base, drift: 1, videoAdvancing: false })).toBe("hold");
    expect(decideAudioSync({ ...base, drift: 1, videoStarved: true })).toBe("hold");
    expect(decideAudioSync({ ...base, held: true, drift: 0.5 })).toBe("wait");
    expect(decideAudioSync({ ...base, held: true, drift: 0.1 })).toBe("resume");
    expect(decideAudioSync({ ...base, held: true, drift: 0.1, videoStarved: true })).toBe("wait");
  });
  it("trims the rate a little", () => {
    expect(trimRate(1, 0.05)).toBe(1);
    expect(trimRate(1, 0.3)).toBeCloseTo(0.964);
    expect(trimRate(1, 2)).toBe(0.95);
    expect(trimRate(1, -2)).toBe(1.05);
  });
});

describe("storyboardFrame", () => {
  const sb = { url: "/yt/s/sb/$M.jpg", width: 160, height: 90, count: 100, cols: 5, rows: 5, interval_ms: 2000 };
  it("finds sheet and tile", () => {
    expect(storyboardFrame(sb, 0, 200)).toMatchObject({ url: "/yt/s/sb/0.jpg", x: 0, y: 0, w: 160, h: 90 });
    expect(storyboardFrame(sb, 13, 200)).toMatchObject({ url: "/yt/s/sb/0.jpg", x: 160, y: 90 });
    expect(storyboardFrame(sb, 60, 200)).toMatchObject({ url: "/yt/s/sb/1.jpg", x: 0, y: 90 });
    expect(storyboardFrame(sb, 10_000, 200)).toMatchObject({ url: "/yt/s/sb/3.jpg", x: 640, y: 360 });
  });
  it("scales and derives the interval from the duration", () => {
    const fr = storyboardFrame({ ...sb, width: 80, height: 45, interval_ms: 0 }, 10, 100, 160)!;
    expect(fr.w).toBe(160);
    expect(fr.sheetW).toBe(800);
    expect(fr.x).toBe(0);
    expect(fr.y).toBe(180);
  });
});

describe("sbStep", () => {
  const seg = (uuid: string, category: SbSegment["category"], s: number, e: number, action: SbSegment["action"] = "skip"): SbSegment => ({
    uuid,
    category,
    action,
    start_s: s,
    end_s: e,
    votes: 1,
  });
  const segs = [seg("a", "sponsor", 10, 20), seg("b", "selfpromo", 30, 40), seg("c", "intro", 0, 5), seg("d", "filler", 50, 60, "mute")];
  const modes: Record<string, "skip" | "ask" | "show" | "off"> = { sponsor: "skip", selfpromo: "ask", intro: "show", filler: "skip" };
  const modeOf = (c: string) => modes[c] ?? "off";
  const none = new Set<string>();
  it("skips, asks, shows and mutes by category", () => {
    expect(sbStep(segs, 12, modeOf, true, none, none).skip?.uuid).toBe("a");
    expect(sbStep(segs, 31, modeOf, true, none, none).ask?.uuid).toBe("b");
    expect(sbStep(segs, 2, modeOf, true, none, none)).toEqual({ skip: null, ask: null, mute: false });
    expect(sbStep(segs, 55, modeOf, true, none, none).mute).toBe(true);
    expect(sbStep(segs, 25, modeOf, true, none, none)).toEqual({ skip: null, ask: null, mute: false });
  });
  it("turns skips into prompts for whitelisted channels", () => {
    const s = sbStep(segs, 12, modeOf, false, none, none);
    expect(s.skip).toBeNull();
    expect(s.ask?.uuid).toBe("a");
  });
  it("honours undo and dismiss until playback leaves", () => {
    const undone = new Set(["a"]);
    expect(sbStep(segs, 12, modeOf, true, undone, none).skip).toBeNull();
    sbForget(segs, 15, undone);
    expect(undone.has("a")).toBe(true);
    sbForget(segs, 21, undone);
    expect(undone.has("a")).toBe(false);
    expect(sbStep(segs, 31, modeOf, true, none, new Set(["b"])).ask).toBeNull();
  });
});

describe("parseVtt", () => {
  const vtt = `WEBVTT
Kind: captions

00:00:01.000 --> 00:00:03.500 align:start position:0%
Hello <c>world</c>

00:00:03.500 --> 00:00:06.000
Hello world
second line

1:02:03.250 --> 1:02:04.000
late &amp; last`;
  it("parses cues, strips tags and rolling repeats", () => {
    const c = parseVtt(vtt);
    expect(c).toHaveLength(3);
    expect(c[0]).toEqual({ start: 1, end: 3.5, text: "Hello world" });
    expect(c[1].text).toBe("second line");
    expect(c[2].start).toBeCloseTo(3723.25);
    expect(c[2].text).toBe("late & last");
  });
  it("finds active cues", () => {
    const c = parseVtt(vtt);
    expect(cuesAt(c, 2).map((x) => x.text)).toEqual(["Hello world"]);
    expect(cuesAt(c, 4).map((x) => x.text)).toEqual(["second line"]);
    expect(cuesAt(c, 100)).toEqual([]);
  });
});

describe("misc", () => {
  it("detects WebKit webviews", () => {
    expect(isWebKit("Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 (KHTML, like Gecko)")).toBe(true);
    expect(isWebKit("Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Chrome/128 Safari/537.36 Edg/128")).toBe(false);
  });
  it("only attenuates loudness", () => {
    expect(loudnessGain(null)).toBe(1);
    expect(loudnessGain(-3)).toBe(1);
    expect(loudnessGain(6)).toBeCloseTo(0.501, 2);
  });
});
