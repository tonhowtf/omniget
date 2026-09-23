// xterm theme built from the app's CSS variables. Tokens are often
// `color-mix(...)`, which xterm cannot parse, so each one is resolved through a
// probe element and a 1x1 canvas into plain `rgba()`.

import type { ITheme } from "@xterm/xterm";

let canvasCtx: CanvasRenderingContext2D | null = null;

function resolveColor(value: string, fallback: string): string {
  if (typeof document === "undefined" || !value) return fallback;
  const probe = document.createElement("span");
  probe.style.color = value;
  probe.style.display = "none";
  document.body.appendChild(probe);
  const computed = getComputedStyle(probe).color;
  probe.remove();
  if (!computed) return fallback;
  if (!canvasCtx) {
    const c = document.createElement("canvas");
    c.width = 1;
    c.height = 1;
    canvasCtx = c.getContext("2d", { willReadFrequently: true });
  }
  if (!canvasCtx) return computed;
  canvasCtx.clearRect(0, 0, 1, 1);
  canvasCtx.fillStyle = fallback;
  canvasCtx.fillStyle = computed;
  canvasCtx.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = canvasCtx.getImageData(0, 0, 1, 1).data;
  return `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)})`;
}

function mix(a: string, b: string, pct: number): string {
  return `color-mix(in srgb, ${a} ${pct}%, ${b})`;
}

export function terminalTheme(el: Element = document.documentElement): ITheme {
  const css = getComputedStyle(el);
  const v = (name: string) => css.getPropertyValue(name).trim();
  const bg = v("--pane-bg") || v("--bg") || "#1c1c1e";
  const fg = v("--text") || "#f5f5f7";
  const red = v("--error") || "#ff453a";
  const green = v("--success") || "#30d158";
  const yellow = v("--warning") || "#ffd60a";
  const blue = v("--blue") || "#0a84ff";
  const magenta = v("--purple") || "#bf5af2";
  const cyan = v("--teal") || "#40c8e0";
  const dim = v("--text-dim") || "#98989d";
  const r = resolveColor;
  return {
    background: r(bg, "#1c1c1e"),
    foreground: r(fg, "#f5f5f7"),
    cursor: r(v("--accent") || fg, "#ff9f0a"),
    cursorAccent: r(bg, "#1c1c1e"),
    selectionBackground: r(v("--selection") || mix(fg, "transparent", 25), "rgba(255,159,10,0.25)"),
    black: r(mix(dim, "black", 40), "#3a3a3c"),
    red: r(red, "#ff453a"),
    green: r(green, "#30d158"),
    yellow: r(yellow, "#ffd60a"),
    blue: r(blue, "#0a84ff"),
    magenta: r(magenta, "#bf5af2"),
    cyan: r(cyan, "#40c8e0"),
    white: r(mix(fg, dim, 70), "#d1d1d6"),
    brightBlack: r(dim, "#8e8e93"),
    brightRed: r(mix(red, "white", 80), "#ff6961"),
    brightGreen: r(mix(green, "white", 80), "#4cd964"),
    brightYellow: r(mix(yellow, "white", 80), "#ffe066"),
    brightBlue: r(mix(blue, "white", 80), "#409cff"),
    brightMagenta: r(mix(magenta, "white", 80), "#da8fff"),
    brightCyan: r(mix(cyan, "white", 80), "#70d7ff"),
    brightWhite: r(fg, "#ffffff"),
  };
}

export function terminalFont(el: Element = document.documentElement): string {
  return getComputedStyle(el).getPropertyValue("--font-mono").trim() || "ui-monospace, Menlo, Consolas, monospace";
}
