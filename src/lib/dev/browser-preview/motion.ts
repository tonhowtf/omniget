type Args = Record<string, unknown>;

const ease = (o: number[], i: number[]) => ({ o: { x: o.slice(0, 1), y: o.slice(1, 2) }, i: { x: i.slice(0, 1), y: i.slice(1, 2) } });
const ease3 = (o: number[], i: number[]) => ({ o: { x: [o[0], o[0], o[0]], y: [o[1], o[1], o[1]] }, i: { x: [i[0], i[0], i[0]], y: [i[1], i[1], i[1]] } });

const SAMPLE_LOTTIE = {
  v: "5.7.4",
  nm: "Preview loader",
  fr: 30,
  ip: 0,
  op: 90,
  w: 240,
  h: 240,
  ddd: 0,
  assets: [],
  layers: [
    {
      ddd: 0,
      ind: 1,
      ty: 4,
      nm: "Square",
      sr: 1,
      ks: {
        o: { a: 0, k: 100 },
        r: { a: 1, k: [{ t: 0, s: [0], ...ease([0.6, 0], [0.2, 1]) }, { t: 45, s: [180], ...ease([0.6, 0], [0.2, 1]) }, { t: 90, s: [360] }] },
        p: { a: 0, k: [120, 120, 0] },
        a: { a: 0, k: [0, 0, 0] },
        s: { a: 1, k: [{ t: 0, s: [60, 60, 100], ...ease3([0.4, 0], [0.2, 1]) }, { t: 45, s: [110, 110, 100], ...ease3([0.4, 0], [0.2, 1]) }, { t: 90, s: [60, 60, 100] }] },
      },
      ao: 0,
      shapes: [
        {
          ty: "gr",
          nm: "Box",
          it: [
            { ty: "rc", nm: "Rect", d: 1, s: { a: 0, k: [90, 90] }, p: { a: 0, k: [0, 0] }, r: { a: 0, k: 18 } },
            { ty: "fl", nm: "Fill", c: { a: 0, k: [1, 0.624, 0.039, 1] }, o: { a: 0, k: 100 }, r: 1 },
            { ty: "tr", p: { a: 0, k: [0, 0] }, a: { a: 0, k: [0, 0] }, s: { a: 0, k: [100, 100] }, r: { a: 0, k: 0 }, o: { a: 0, k: 100 }, sk: { a: 0, k: 0 }, sa: { a: 0, k: 0 } },
          ],
        },
      ],
      ip: 0,
      op: 90,
      st: 0,
      bm: 0,
    },
    {
      ddd: 0,
      ind: 2,
      ty: 4,
      nm: "Ring",
      sr: 1,
      ks: {
        o: { a: 1, k: [{ t: 0, s: [100], ...ease([0.4, 0], [0.6, 1]) }, { t: 60, s: [0] }] },
        r: { a: 0, k: 0 },
        p: { a: 0, k: [120, 120, 0] },
        a: { a: 0, k: [0, 0, 0] },
        s: { a: 1, k: [{ t: 0, s: [50, 50, 100], ...ease3([0.2, 0], [0.2, 1]) }, { t: 60, s: [160, 160, 100] }] },
      },
      ao: 0,
      shapes: [
        {
          ty: "gr",
          nm: "Circle",
          it: [
            { ty: "el", nm: "Ellipse", d: 1, s: { a: 0, k: [120, 120] }, p: { a: 0, k: [0, 0] } },
            { ty: "st", nm: "Stroke", c: { a: 0, k: [0.369, 0.859, 0.784, 1] }, o: { a: 0, k: 100 }, w: { a: 0, k: 6 }, lc: 2, lj: 2 },
            { ty: "tr", p: { a: 0, k: [0, 0] }, a: { a: 0, k: [0, 0] }, s: { a: 0, k: [100, 100] }, r: { a: 0, k: 0 }, o: { a: 0, k: 100 }, sk: { a: 0, k: 0 }, sa: { a: 0, k: 0 } },
          ],
        },
      ],
      ip: 0,
      op: 90,
      st: 0,
      bm: 0,
    },
  ],
};

const SAMPLE_TIMELINE = {
  version: 1,
  name: "Saved bounce",
  duration: 1600,
  stage: { kind: "text", text: "Hello" },
  tracks: {
    x: [],
    y: [
      { time: 0, value: -80, ease: [0.55, 0, 1, 0.45] },
      { time: 600, value: 60, ease: [0, 0.55, 0.45, 1] },
      { time: 1000, value: 0, ease: [0.42, 0, 0.58, 1] },
    ],
    scale: [{ time: 0, value: 1, ease: [0.34, 1.56, 0.64, 1] }, { time: 1600, value: 1.4, ease: [0.42, 0, 0.58, 1] }],
    rotate: [],
    opacity: [{ time: 0, value: 0, ease: [0, 0, 0.58, 1] }, { time: 300, value: 1, ease: [0, 0, 0.58, 1] }],
    color: [{ time: 0, value: "#c77dff", ease: [0.42, 0, 0.58, 1] }, { time: 1600, value: "#5aa9ff", ease: [0.42, 0, 0.58, 1] }],
  },
};

const LOGO_SVG =
  "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><circle cx='50' cy='50' r='44' fill='#5EDBC8'/><path d='M30 52l14 14 28-32' fill='none' stroke='#fff' stroke-width='10' stroke-linecap='round' stroke-linejoin='round'/></svg>";

let recent: { id: string; name: string; saved_at_ms: number; timeline: string }[] = [
  { id: "r1", name: "Card entrance", saved_at_ms: Date.now() - 3_600_000, timeline: JSON.stringify({ ...SAMPLE_TIMELINE, name: "Card entrance" }) },
];

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const name = (p: string) => p.split("/").pop() ?? p;

export const motionFixtures: Record<string, (args: Args) => unknown> = {
  motion_lottie_open: async ({ path }) => {
    const p = String(path);
    await sleep(350);
    if (p.includes("broken")) throw "The .lottie is not a valid zip archive (invalid Zip archive: Could not find EOCD).";
    const dot = p.endsWith(".lottie");
    const animation = JSON.stringify(SAMPLE_LOTTIE);
    return {
      path: p,
      name: name(p),
      kind: dot ? "dotlottie" : "json",
      size_bytes: animation.length,
      animation,
      animation_id: dot ? "loader" : null,
      animation_count: dot ? 2 : 1,
      manifest: dot ? { version: "1", generator: "preview", animations: [{ id: "loader" }, { id: "success" }] } : null,
      embedded_images: 0,
    };
  },
  motion_png_save: async ({ path, dataBase64 }) => {
    await sleep(250);
    return { path: String(path), size_bytes: Math.round((String(dataBase64).length * 3) / 4) };
  },
  motion_timeline_save: async ({ path, json }) => {
    await sleep(200);
    return { path: String(path), size_bytes: String(json).length };
  },
  motion_timeline_load: async ({ path }) => {
    await sleep(200);
    if (String(path).includes("broken")) throw "This JSON is not an OmniGet Motion timeline (version 1 with tracks).";
    return JSON.stringify(SAMPLE_TIMELINE);
  },
  motion_image_open: async ({ path }) => {
    await sleep(150);
    return { name: name(String(path)), mime: "image/svg+xml", data_url: `data:image/svg+xml;utf8,${encodeURIComponent(LOGO_SVG)}`, size_bytes: LOGO_SVG.length };
  },
  motion_recent_list: () => recent,
  motion_recent_save: ({ id, name: n, json }) => {
    const label = String(n ?? "").trim();
    if (!label) throw "Give the timeline a name before keeping it in Recent.";
    const keep = typeof id === "string" && recent.some((r) => r.id === id) ? id : `r${Date.now()}`;
    recent = [{ id: keep, name: label, saved_at_ms: Date.now(), timeline: String(json) }, ...recent.filter((r) => r.id !== keep)].slice(0, 8);
    return recent;
  },
  motion_recent_remove: ({ id }) => {
    recent = recent.filter((r) => r.id !== id);
    return recent;
  },
};

export function motionSaveDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/motion")) return undefined;
  const o = (options ?? {}) as { defaultPath?: string };
  return `/preview/Motion/${o.defaultPath ?? "export"}`;
}

export function motionDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/motion")) return undefined;
  const filters = (options?.filters as { extensions?: string[] }[] | undefined) ?? [];
  const exts = filters.flatMap((f) => f.extensions ?? []);
  if (exts.includes("lottie")) return "/preview/Motion/loader.lottie";
  if (exts.includes("svg")) return "/preview/Pictures/logo.svg";
  return "/preview/Motion/bounce-timeline.json";
}
