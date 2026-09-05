// WCAG contrast audit for OmniGet themes.
// Computes ratio for: text-muted/surface, text-muted/surface-hi, text-dim/surface.

const themes = [
  { name: "dark",                 secondary: "#F5F5F7", tertiary: "#98989D", button: "#2A2A2D", buttonElev: "#38383B", primary: "#232325" },
  { name: "light",                secondary: "#1D1D1F", tertiary: "#6E6A64", button: "#FFFFFF", buttonElev: "#F5F3EE", primary: "#FBFAF7" },
  { name: "catppuccin-mocha",     secondary: "#cdd6f4", tertiary: "#a6adc8", button: "#313244", buttonElev: "#45475a", primary: "#1e1e2e" },
  { name: "catppuccin-macchiato", secondary: "#cad3f5", tertiary: "#a5adcb", button: "#363a4f", buttonElev: "#494d64", primary: "#24273a" },
  { name: "catppuccin-frappe",    secondary: "#c6d0f5", tertiary: "#c3cdf1", button: "#414559", buttonElev: "#51576d", primary: "#303446" },
  { name: "catppuccin-latte",     secondary: "#4c4f69", tertiary: "#3e4159", button: "#ccd0da", buttonElev: "#bcc0cc", primary: "#eff1f5" },
  { name: "one-dark-pro",         secondary: "#abb2bf", tertiary: "#a9b0bd", button: "#2c313a", buttonElev: "#3e4452", primary: "#282c34" },
  { name: "dracula",              secondary: "#F8F8F2", tertiary: "#b6b1cc", button: "#2B2640", buttonElev: "#424450", primary: "#22212C" },
  { name: "nyxvamp-veil",         secondary: "#D9E0EE", tertiary: "#9594ab", button: "#2E2E3E", buttonElev: "#302D41", primary: "#1E1E2E" },
  { name: "nyxvamp-obsidian",     secondary: "#C0C0CE", tertiary: "#87859b", button: "#1E1E20", buttonElev: "#1E1E20", primary: "#000A0F" },
  { name: "nyxvamp-radiance",     secondary: "#1E1E2E", tertiary: "#5A5570", button: "#E8E8F0", buttonElev: "#E8D5FF", primary: "#F7F7FF" },
  { name: "eink-day",             secondary: "#1d1d1b", tertiary: "#5c5a56", button: "#ebe5d7", buttonElev: "#ebe5d7", primary: "#f5f2ea" },
  { name: "eink-sepia",           secondary: "#3f2e20", tertiary: "#6e5440", button: "#e6dcc3", buttonElev: "#e6dcc3", primary: "#f0e6d2" },
  { name: "eink-night",           secondary: "#d4d4d0", tertiary: "#8e8e88", button: "#121212", buttonElev: "#121212", primary: "#0a0a0a" },
];

function hexToRgb(h) {
  h = h.replace("#", "");
  return [parseInt(h.slice(0,2),16), parseInt(h.slice(2,4),16), parseInt(h.slice(4,6),16)];
}

function rgbToHex(rgb) {
  return "#" + rgb.map(c => Math.round(c).toString(16).padStart(2, "0")).join("");
}

function mix(a, b, percent) {
  const ra = hexToRgb(a), rb = hexToRgb(b);
  const p = percent / 100;
  return rgbToHex([
    ra[0]*p + rb[0]*(1-p),
    ra[1]*p + rb[1]*(1-p),
    ra[2]*p + rb[2]*(1-p),
  ]);
}

function linearize(c) {
  c = c / 255;
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function luminance(hex) {
  const [r, g, b] = hexToRgb(hex);
  return 0.2126*linearize(r) + 0.7152*linearize(g) + 0.0722*linearize(b);
}

function ratio(a, b) {
  const la = luminance(a), lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

const TARGET = 4.5;

console.log(`| theme | text-muted | muted/surf | muted/surf-hi | dim/surf | overall |`);
console.log(`|---|---|---|---|---|---|`);
const failing = [];
for (const t of themes) {
  const muted = mix(t.secondary, t.tertiary, 65);
  const r1 = ratio(muted, t.button);
  const r2 = ratio(muted, t.buttonElev);
  const r3 = ratio(t.tertiary, t.button);
  const passing = r1 >= TARGET && r2 >= TARGET;
  const overall = passing ? "PASS" : "FAIL";
  console.log(`| ${t.name} | ${muted} | ${r1.toFixed(2)} | ${r2.toFixed(2)} | ${r3.toFixed(2)} | ${overall} |`);
  if (!passing) failing.push({ ...t, muted, r1, r2 });
}

console.log("\n--- Per-theme tertiary fix proposals (target text-dim ≥ 4.5:1 on surface) ---");
for (const t of themes) {
  const r3 = ratio(t.tertiary, t.button);
  if (r3 < 4.5) {
    let bestTertiary = t.tertiary;
    let bestRatio = r3;
    for (let step = 1; step <= 30; step++) {
      const candidate = mix(t.secondary, t.tertiary, step * 3);
      const r = ratio(candidate, t.button);
      const r2 = ratio(candidate, t.buttonElev);
      if (r >= 4.5 && r2 >= 4.5) {
        const newMuted = mix(t.secondary, candidate, 65);
        const rm1 = ratio(newMuted, t.button);
        const rm2 = ratio(newMuted, t.buttonElev);
        console.log(`  ${t.name}: --tertiary ${t.tertiary} → ${candidate} (dim/surf=${r.toFixed(2)}:1, dim/surf-hi=${r2.toFixed(2)}:1, muted/surf=${rm1.toFixed(2)}, muted/surf-hi=${rm2.toFixed(2)})`);
        bestTertiary = candidate;
        break;
      }
    }
  }
}

console.log("\n--- Failing muted themes (text-muted-only failure case) ---");
for (const t of failing) {
  // Strategy: for dark themes, lighten tertiary toward secondary until muted hits target.
  // For light themes, darken tertiary toward secondary until target.
  // Simulate by adjusting tertiary; muted = mix(secondary, tertiary, 65%).
  // Try increasing/decreasing tertiary brightness in steps.
  const isDark = luminance(t.button) < luminance(t.secondary);
  let bestTertiary = t.tertiary;
  let bestRatio = t.r1;
  // Try 20 steps blending tertiary toward secondary
  for (let step = 1; step <= 20; step++) {
    const candidate = mix(t.secondary, t.tertiary, step * 5); // step*5% secondary
    const candidateMuted = mix(t.secondary, candidate, 65);
    const r = ratio(candidateMuted, t.button);
    if (r >= TARGET && r > bestRatio) {
      bestTertiary = candidate;
      bestRatio = r;
      const r2c = ratio(candidateMuted, t.buttonElev);
      if (r2c >= TARGET) {
        console.log(`  ${t.name}: tertiary ${t.tertiary} → ${candidate} (muted=${candidateMuted}, surf=${r.toFixed(2)}:1, surf-hi=${r2c.toFixed(2)}:1)`);
        break;
      }
    }
  }
}
