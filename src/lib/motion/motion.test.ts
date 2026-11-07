import { describe, expect, it } from "vitest";
import { PRESETS, Y_MAX, Y_MIN, bezierEase, clampBezier, findPreset, moveHandle, overshoots, type Bezier } from "./bezier";
import { cssBezier, gsapEase, motionEase, motionSpring, svelteEase, svelteSpring, swiftSpring } from "./exports";
import { linearEase, linearStops, sampleFn, simplify, toLinear } from "./linear";
import { DEFAULT_SPRING, fromSwiftUI, simulateSpring, springAt, toSvelteSpring, toSwiftUI } from "./spring";
import { frameAt, mixColor, parseTimeline, safeName, sampleTimeline, serializeTimeline, toCssKeyframes, toWaapi, valueAt, type Keyframe } from "./timeline";
import { pngName, readLottie, timeLabel } from "./lottie";

const EASE: Bezier = [0.25, 0.1, 0.25, 1];

describe("bezier", () => {
  it("matches known values of CSS ease", () => {
    const f = bezierEase(EASE);
    expect(f(0)).toBe(0);
    expect(f(1)).toBe(1);
    expect(f(0.5)).toBeCloseTo(0.8024, 3);
    expect(f(0.25)).toBeCloseTo(0.4085, 3);
  });

  it("is symmetric for ease-in-out", () => {
    const f = bezierEase([0.42, 0, 0.58, 1]);
    expect(f(0.5)).toBeCloseTo(0.5, 5);
    expect(f(0.2) + f(0.8)).toBeCloseTo(1, 5);
  });

  it("returns identity for linear", () => {
    const f = bezierEase([0, 0, 1, 1]);
    expect(f(0.37)).toBeCloseTo(0.37, 6);
  });

  it("overshoots with back curves and solves flat slopes", () => {
    const back = bezierEase([0.34, 1.56, 0.64, 1]);
    const peak = Math.max(...Array.from({ length: 101 }, (_, i) => back(i / 100)));
    expect(peak).toBeGreaterThan(1);
    const flat = bezierEase([1, 0, 0, 1]);
    expect(flat(0.5)).toBeCloseTo(0.5, 3);
    expect(flat(0.01)).toBeGreaterThanOrEqual(0);
  });

  it("clamps x to 0..1 and y to the editor range", () => {
    expect(clampBezier([-1, -3, 2, 3])).toEqual([0, Y_MIN, 1, Y_MAX]);
    expect(overshoots([0.68, -0.6, 0.32, 1.6])).toBe(true);
    expect(overshoots(EASE)).toBe(false);
  });

  it("recognises presets", () => {
    expect(findPreset([0.42, 0, 1, 1])?.id).toBe("ease-in");
    expect(PRESETS.every((p) => p.value[0] >= 0 && p.value[0] <= 1 && p.value[2] >= 0 && p.value[2] <= 1)).toBe(true);
  });

  it("keeps every preset inside the editor range", () => {
    for (const p of PRESETS) expect(clampBezier(p.value)).toEqual(p.value);
  });

  it("moves one handle without touching the other", () => {
    const back: Bezier = [0.68, -0.6, 0.32, 1.6];
    expect(moveHandle(back, 0, 0.69, -0.6)).toEqual([0.69, -0.6, 0.32, 1.6]);
    expect(moveHandle(back, 1, 1.4, 9)).toEqual([0.68, -0.6, 1, Y_MAX]);
    expect(moveHandle(back, 0, 0.12345, 0.5)).toEqual([0.123, 0.5, 0.32, 1.6]);
  });
});

describe("easing exports", () => {
  it("formats every target", () => {
    expect(cssBezier(EASE)).toBe("cubic-bezier(0.25, 0.1, 0.25, 1)");
    expect(motionEase(EASE, 600)).toBe("{ duration: 0.6, ease: [0.25, 0.1, 0.25, 1] }");
    expect(gsapEase(EASE)).toBe('CustomEase.create("custom", "M0,0 C0.25,0.1 0.25,1 1,1");');
  });

  it("emits a Svelte easing function that matches the solver", () => {
    const code = svelteEase(EASE).replace("export function ease", "return function ease");
    const fn = new Function(code)() as (t: number) => number;
    const ref = bezierEase(EASE);
    for (const t of [0, 0.1, 0.33, 0.5, 0.9, 1]) expect(fn(t)).toBeCloseTo(ref(t), 4);
  });
});

describe("spring", () => {
  it("settles to 1 within tolerance", () => {
    const r = simulateSpring(DEFAULT_SPRING);
    expect(r.settled).toBe(true);
    expect(r.settleMs).toBeGreaterThan(500);
    expect(r.settleMs).toBeLessThan(2000);
    expect(r.max).toBeGreaterThan(1);
    expect(springAt(r, r.settleMs + 100)).toBe(1);
    expect(r.samples[0]).toEqual({ t: 0, x: 0 });
  });

  it("does not overshoot when critically damped", () => {
    const r = simulateSpring({ stiffness: 100, damping: 20, mass: 1, velocity: 0 });
    expect(r.dampingRatio).toBeCloseTo(1, 6);
    expect(r.max).toBeLessThanOrEqual(1 + 1e-6);
    expect(r.settled).toBe(true);
  });

  it("matches the analytic underdamped solution", () => {
    const p = { stiffness: 170, damping: 26, mass: 1, velocity: 0 };
    const r = simulateSpring(p, 1);
    const w0 = Math.sqrt(170);
    const z = 26 / (2 * w0);
    const wd = w0 * Math.sqrt(1 - z * z);
    const t = 0.2;
    const exact = 1 - Math.exp(-z * w0 * t) * (Math.cos(wd * t) + ((z * w0) / wd) * Math.sin(wd * t));
    expect(springAt(r, 200)).toBeCloseTo(exact, 4);
  });

  it("reports undamped springs as not settled", () => {
    const r = simulateSpring({ stiffness: 100, damping: 0, mass: 1, velocity: 0 }, 20);
    expect(r.settled).toBe(false);
  });

  it("converts to Svelte and SwiftUI and back", () => {
    const s = toSvelteSpring(DEFAULT_SPRING);
    expect(s.stiffness).toBeCloseTo(100 / 3600, 6);
    expect(s.damping).toBeCloseTo(10 / 60, 6);
    expect(s.clamped).toBe(false);
    expect(toSvelteSpring({ stiffness: 10000, damping: 100, mass: 1, velocity: 0 }).clamped).toBe(true);
    const sw = toSwiftUI({ stiffness: 170, damping: 26, mass: 1, velocity: 0 });
    const back = fromSwiftUI(sw);
    expect(back.stiffness).toBeCloseTo(170, 6);
    expect(back.damping).toBeCloseTo(26, 6);
    expect(motionSpring(DEFAULT_SPRING)).toBe('{ type: "spring", stiffness: 100, damping: 10, mass: 1 }');
    expect(motionSpring({ ...DEFAULT_SPRING, velocity: 5 })).not.toContain("velocity");
    expect(svelteSpring(DEFAULT_SPRING)).toContain("stiffness: 0.0278");
    expect(swiftSpring(DEFAULT_SPRING)).toMatch(/^\.spring\(response: 0\.628, dampingFraction: 0\.5\)$/);
  });
});

describe("linear()", () => {
  it("starts at 0 and ends at 1", () => {
    const r = simulateSpring(DEFAULT_SPRING);
    const css = toLinear(r.samples, 0.002);
    expect(css.startsWith("linear(0, ")).toBe(true);
    expect(css.endsWith(", 1)")).toBe(true);
    const stops = linearStops(r.samples, 0.002);
    expect(stops[0]).toEqual({ t: 0, x: 0 });
    expect(stops[stops.length - 1]).toEqual({ t: 1, x: 1 });
  });

  it("stays within tolerance of the source curve", () => {
    const fn = bezierEase([0.16, 1, 0.3, 1]);
    const pts = sampleFn(fn, 400);
    const stops = linearStops(pts, 0.005);
    expect(stops.length).toBeLessThan(pts.length / 4);
    const approx = linearEase(stops);
    for (const p of pts) expect(Math.abs(approx(p.t) - p.x)).toBeLessThanOrEqual(0.0051);
  });

  it("drops collinear points", () => {
    const pts = [0, 0.25, 0.5, 0.75, 1].map((t) => ({ t, x: t }));
    expect(simplify(pts, 0.001)).toEqual([pts[0], pts[4]]);
    expect(toLinear(pts)).toBe("linear(0, 1)");
  });
});

describe("timeline", () => {
  const k = (time: number, value: number | string): Keyframe => ({ id: String(time), time, value, ease: [0, 0, 1, 1] });

  it("interpolates numbers and colors", () => {
    const track = [k(0, 0), k(1000, 100)];
    expect(valueAt("x", track, -10)).toBe(0);
    expect(valueAt("x", track, 500)).toBeCloseTo(50, 6);
    expect(valueAt("x", track, 2000)).toBe(100);
    expect(valueAt("x", [], 10)).toBe(0);
    expect(valueAt("scale", [], 10)).toBe(1);
    expect(mixColor("#000000", "#ffffff", 0.5)).toBe("#808080");
    expect(valueAt("color", [k(0, "#ff0000"), k(100, "#0000ff")], 50)).toBe("#800080");
  });

  it("applies segment easing", () => {
    const track: Keyframe[] = [{ id: "a", time: 0, value: 0, ease: EASE }, k(1000, 1)];
    expect(Number(valueAt("opacity", track, 500))).toBeCloseTo(0.8024, 3);
  });

  it("builds a frame from all tracks", () => {
    const f = frameAt(sampleTimeline(), 0);
    expect(f.x).toBe(-160);
    expect(f.opacity).toBe(0);
    expect(f.y).toBe(0);
  });

  it("exports CSS keyframes and WAAPI", () => {
    const tl = sampleTimeline();
    const css = toCssKeyframes(tl, { name: "My Box", iterations: "infinite", fill: false });
    expect(css).toContain("@keyframes my-box-x {");
    expect(css).toContain("@property --motion-x");
    expect(css).toContain("0% { --motion-x: -160px; animation-timing-function: cubic-bezier(0.16, 1, 0.3, 1); }");
    expect(css).toContain("100% { background-color: #ff9f0a; }");
    expect(css).toContain("my-box-opacity 2000ms linear infinite");
    expect(css).toContain("30% { scale: 1; }");
    const js = toWaapi(tl, { iterations: 2, fill: true, selector: ".box" });
    expect(js).toContain('document.querySelector(".box")');
    expect(js).toContain('{ offset: 0.45, "--motion-x": "160px", easing: "cubic-bezier(0.42, 0, 0.58, 1)" }');
    expect(js).toContain("iterations: 2, fill: \"both\"");
    expect(safeName("123")).toBe("motion-123");
    expect(css).toContain("animation-timing-function: cubic-bezier(0.34, 1.56, 0.64, 1)");
  });

  it("exports what the stage shows for every preset", () => {
    const tl = sampleTimeline();
    for (const p of PRESETS) {
      tl.tracks.scale[0].ease = [...p.value];
      expect(toCssKeyframes(tl, { name: "b", iterations: 1, fill: false })).toContain(cssBezier(p.value));
      const parsed = parseTimeline(serializeTimeline(tl));
      expect(parsed.ok && findPreset(parsed.timeline.tracks.scale[0].ease)?.id).toBe(p.id);
    }
  });

  it("leaves color out of image exports and flags the dropped image", () => {
    const tl = sampleTimeline();
    tl.stage.kind = "image";
    expect(toCssKeyframes(tl, { name: "img", iterations: 1, fill: false })).not.toContain("color");
    expect(toWaapi(tl, { iterations: 1, fill: false, selector: ".img" })).not.toContain("Color");
    const parsed = parseTimeline(serializeTimeline(tl));
    expect(parsed.ok && parsed.imageDropped).toBe(true);
    expect(parsed.ok && parsed.timeline.stage.kind).toBe("box");
    const box = parseTimeline(serializeTimeline(sampleTimeline()));
    expect(box.ok && box.imageDropped).toBe(false);
  });

  it("keeps an empty name empty so the UI can show its own placeholder", () => {
    const tl = sampleTimeline();
    expect(tl.name).toBe("");
    const parsed = parseTimeline(serializeTimeline(tl));
    expect(parsed.ok && parsed.timeline.name).toBe("");
    expect(safeName("")).toBe("motion-timeline");
  });

  it("round-trips through JSON and rejects bad input", () => {
    const tl = sampleTimeline();
    const parsed = parseTimeline(serializeTimeline(tl));
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.timeline.tracks.x.map((x) => x.value)).toEqual([-160, 160, -160]);
      expect(parsed.timeline.duration).toBe(2000);
    }
    expect(parseTimeline("nope")).toEqual({ ok: false, error: "not_object" });
    expect(parseTimeline('{"version":2}')).toEqual({ ok: false, error: "bad_version" });
    expect(parseTimeline('{"version":1,"tracks":{"color":[{"time":0,"value":"red"}]}}')).toEqual({ ok: false, error: "bad_tracks" });
  });
});

describe("lottie", () => {
  it("reads info", () => {
    const r = readLottie(JSON.stringify({ v: "5.7.0", nm: "Hi", fr: 30, ip: 0, op: 90, w: 200, h: 100, layers: [{}, {}], assets: [{ id: "a", p: "img.png" }] }));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.info.durationMs).toBe(3000);
      expect(r.info.layers).toBe(2);
      expect(r.info.images).toBe(1);
    }
    expect(readLottie("{").ok).toBe(false);
    expect(readLottie('{"a":1}')).toEqual({ ok: false, error: "not_lottie" });
    expect(readLottie('{"fr":30,"ip":0,"op":0,"w":1,"h":1,"layers":[]}')).toEqual({ ok: false, error: "no_frames" });
    expect(pngName("loader.lottie", 12.4)).toBe("loader-frame-12.png");
    expect(timeLabel(1234)).toBe("1.23");
  });
});
