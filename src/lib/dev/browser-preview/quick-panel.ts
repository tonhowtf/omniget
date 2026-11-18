type Args = Record<string, unknown>;

type Snippet = { id: number; name: string; text: string };
type Clip = { id: number; preview: string; chars: number; pinned: boolean };

let awake = { mode: "1h", active: true, until_ms: Date.now() + 42 * 60_000, remaining_secs: 42 * 60 };
let clipsOn = true;
let clips: Clip[] = [
  { id: 4, preview: "https://example.com/watch?v=dQw4w9WgXcQ", chars: 39, pinned: true },
  { id: 9, preview: "git rebase --onto main feature~3 feature", chars: 40, pinned: false },
  { id: 8, preview: "Thanks for the update. I will send the signed copy by Friday.", chars: 61, pinned: false },
  { id: 7, preview: "#FF9F0A", chars: 7, pinned: false },
  { id: 6, preview: "SELECT id, title FROM downloads WHERE finished_at IS NULL ORDER BY id DESC LIMIT 20;", chars: 84, pinned: false },
];
let snippets: Snippet[] = [
  { id: 1, name: "Email signature", text: "Best regards,\nAna Souza\nana@example.com" },
  { id: 2, name: "Shipping address", text: "221B Baker Street, London NW1 6XE" },
];
let targets = [
  { id: "4242", name: "Browser", pid: 4242, volume: 70, muted: false },
  { id: "5120", name: "Music player", pid: 5120, volume: 45, muted: false },
  { id: "6031", name: "Video call", pid: 6031, volume: 100, muted: true },
  { id: "0", name: "System sounds", pid: 0, volume: 60, muted: false },
];
let tick = 0;

function clipView() {
  return { enabled: clipsOn, entries: [...clips.filter((c) => c.pinned), ...clips.filter((c) => !c.pinned)] };
}

function wave(base: number, spread: number, phase = 0): number {
  return Math.round((base + Math.sin(tick / 3 + phase) * spread) * 10) / 10;
}

export const quickPanelFixtures: Record<string, (args: Args) => unknown> = {
  quicktools_status: () => ({
    capabilities: { clipboard_history: true, volume: "apps" },
    awake,
    clipboard_enabled: clipsOn,
    snippets,
  }),
  quicktools_awake_set: ({ mode }) => {
    const minutes = { "30m": 30, "1h": 60, "2h": 120 }[String(mode)];
    awake = {
      mode: String(mode),
      active: mode !== "off",
      until_ms: minutes ? Date.now() + minutes * 60_000 : (null as unknown as number),
      remaining_secs: minutes ? minutes * 60 : (null as unknown as number),
    };
    return awake;
  },
  quicktools_system_stats: () => {
    tick++;
    return {
      cpu_percent: wave(23, 9),
      memory_used: 11.4 * 1024 ** 3 + Math.sin(tick / 5) * 2e8,
      memory_total: 16 * 1024 ** 3,
      disk_free: 182.6 * 1024 ** 3,
      disk_total: 494 * 1024 ** 3,
      net_down_per_s: Math.round(2.4 * 1024 * 1024 + Math.sin(tick / 2) * 6e5),
      net_up_per_s: Math.round(180 * 1024 + Math.cos(tick / 2) * 4e4),
      top: [
        { pid: 4242, name: "browser", cpu_percent: wave(9.2, 2) },
        { pid: 812, name: "WindowServer", cpu_percent: wave(4.1, 1, 1) },
        { pid: 5120, name: "Music player", cpu_percent: wave(2.6, 0.6, 2) },
        { pid: 977, name: "omniget", cpu_percent: wave(1.3, 0.3, 3) },
        { pid: 6031, name: "video-call", cpu_percent: wave(0.9, 0.2, 4) },
      ],
    };
  },
  quicktools_volume_list: () => targets,
  quicktools_volume_set: ({ id, volume, muted }) => {
    targets = targets.map((t) =>
      t.id === id ? { ...t, volume: typeof volume === "number" ? volume : t.volume, muted: typeof muted === "boolean" ? muted : t.muted } : t,
    );
    return null;
  },
  quicktools_snippet_save: ({ id, name, text }) => {
    const label = String(name ?? "").trim();
    if (!label || !String(text ?? "").trim()) {
      throw `INVALID_INPUT: ${JSON.stringify({ message: "name must have 1 to 60 characters", retryable: false, next_step: "fix the argument named in the message and call again" })}`;
    }
    if (typeof id === "number") snippets = snippets.map((s) => (s.id === id ? { id, name: label, text: String(text) } : s));
    else snippets = [...snippets, { id: Math.max(0, ...snippets.map((s) => s.id)) + 1, name: label, text: String(text) }];
    return snippets;
  },
  quicktools_snippet_delete: ({ id }) => {
    snippets = snippets.filter((s) => s.id !== id);
    return snippets;
  },
  quicktools_snippet_copy: () => null,
  quicktools_clipboard: ({ enabled }) => {
    if (typeof enabled === "boolean") {
      clipsOn = enabled;
      if (!enabled) clips = [];
    }
    return clipView();
  },
  quicktools_clipboard_act: ({ op, id }) => {
    if (op === "clear") clips = clips.filter((c) => c.pinned);
    if (op === "delete") clips = clips.filter((c) => c.id !== id);
    if (op === "pin" || op === "unpin") clips = clips.map((c) => (c.id === id ? { ...c, pinned: op === "pin" } : c));
    if (op === "copy") {
      const hit = clips.find((c) => c.id === id);
      if (hit) clips = [hit, ...clips.filter((c) => c.id !== id)];
    }
    return clipView();
  },
  quicktools_open: () => null,
  quicktools_close: () => null,
};
