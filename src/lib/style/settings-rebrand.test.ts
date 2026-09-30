import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const appCss = readFileSync("src/app.css", "utf8");
const css = readFileSync("src/lib/style/settings-rebrand.css", "utf8");

function block(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/--sr-([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

const SPEC = {
  light: { canvas: "#F6F7F9", surface: "#FFFFFF", "surface-hover": "#ECEEF2", text: "#20242B", muted: "#59616F", border: "#D8DDE5", "control-border": "#7B8493", accent: "#AE4600", "on-accent": "#FFFFFF", "accent-soft": "#FFF0E4", danger: "#B3261E", success: "#176A3A", focus: "#005FCC" },
  dark: { canvas: "#181A1F", surface: "#202329", "surface-hover": "#2B2F37", text: "#F3F4F6", muted: "#B4BBC7", border: "#454D5B", "control-border": "#8792A3", accent: "#FFAE70", "on-accent": "#251507", "accent-soft": "#3E2A1D", danger: "#FFAAA3", success: "#88D5A8", focus: "#88BBFF" },
} as const;

function luminance(hex: string) {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
    .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
}
function contrast(a: string, b: string) {
  const [low, high] = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (high + 0.05) / (low + 0.05);
}

describe("settings rebrand tokens", () => {
  let spacing: Readonly<Record<string, number>>;
  beforeAll(async () => {
    vi.stubGlobal("$state", <T>(value: T) => value);
    spacing = (await import("../stores/settings-store.svelte")).SPACING_BASE_PX;
  });

  it("uses one spacing base: app.css :root equals the store base, 4/8/12/16/20/28/40/56/80", () => {
    const fromCss = Object.fromEntries([...appCss.matchAll(/(--space-[1-9]):\s*([\d.]+)px;/g)].map((m) => [m[1], Number(m[2])]));
    expect(fromCss).toEqual(spacing);
    expect(Object.values(spacing)).toEqual([4, 8, 12, 16, 20, 28, 40, 56, 80]);
  });

  for (const mode of ["light", "dark"] as const) {
    it(`omniget-desktop ${mode} carries the specified values`, () => {
      expect(block(`:root[data-preset="omniget-desktop"][data-theme="${mode}"]`)).toMatchObject(SPEC[mode]);
    });

    it(`omniget-desktop ${mode} meets the specified contrast pairs`, () => {
      const c = SPEC[mode];
      for (const [fg, bg, min] of [
        [c.text, c.surface, 4.5], [c.muted, c.surface, 4.5], [c.muted, c.canvas, 4.5], [c["on-accent"], c.accent, 4.5],
        [c.danger, c.surface, 4.5], [c.success, c.surface, 4.5], [c.focus, c.surface, 3], [c["control-border"], c.surface, 3],
      ] as const) expect(contrast(fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(min);
    });
  }

  it("declares radii 6/10/14, durations 120/160 and reduced motion from the OS or the preference", () => {
    expect(css).toContain("--sr-radius-control: 6px");
    expect(css).toContain("--sr-radius-group: 10px");
    expect(css).toContain("--sr-radius-dialog: 14px");
    expect(css).toContain("--sr-duration-color: 120ms");
    expect(css).toContain("--sr-duration-disclosure: 160ms");
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
    expect(css).toContain('[data-reduce-motion="true"]');
    expect(appCss).toMatch(/@media \(prefers-reduced-motion: reduce\) \{\s*:root \{\s*--duration-fast: 0\.01ms/);
    expect(appCss).not.toContain("reduce-motion-pulse");
    expect(appCss).toContain("@import './lib/style/settings-rebrand.css';");
  });
});
