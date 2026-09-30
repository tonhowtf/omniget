// Terminal colour schemes for Settings → Canvas → Terminal theme. `system`
// follows the app's CSS tokens (see $components/central/terminal/theme);
// the others are fixed schemes (iTerm2 / Ghostty classics).

export interface TermScheme {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  /** 16 ANSI colours: black, red, green, yellow, blue, magenta, cyan, white, then bright. */
  ansi: string[];
}

export const TERM_SCHEMES: Record<string, TermScheme> = {
  dark: {
    background: '#1c1c1e',
    foreground: '#f5f5f7',
    cursor: '#ff9f0a',
    selectionBackground: 'rgba(255,159,10,0.3)',
    ansi: ['#3a3a3c', '#ff453a', '#30d158', '#ffd60a', '#0a84ff', '#bf5af2', '#40c8e0', '#d1d1d6', '#8e8e93', '#ff6961', '#4cd964', '#ffe066', '#409cff', '#da8fff', '#70d7ff', '#ffffff'],
  },
  light: {
    background: '#ffffff',
    foreground: '#1d1d1f',
    cursor: '#007aff',
    selectionBackground: 'rgba(0,122,255,0.25)',
    ansi: ['#1d1d1f', '#d70015', '#248a3d', '#a05a00', '#0040dd', '#8944ab', '#0071a4', '#8e8e93', '#6e6e73', '#ff3b30', '#34c759', '#c93400', '#007aff', '#af52de', '#32ade6', '#3a3a3c'],
  },
  dracula: {
    background: '#282a36',
    foreground: '#f8f8f2',
    cursor: '#f8f8f2',
    selectionBackground: 'rgba(68,71,90,0.9)',
    ansi: ['#21222c', '#ff5555', '#50fa7b', '#f1fa8c', '#bd93f9', '#ff79c6', '#8be9fd', '#f8f8f2', '#6272a4', '#ff6e6e', '#69ff94', '#ffffa5', '#d6acff', '#ff92df', '#a4ffff', '#ffffff'],
  },
  nord: {
    background: '#2e3440',
    foreground: '#d8dee9',
    cursor: '#d8dee9',
    selectionBackground: 'rgba(67,76,94,0.9)',
    ansi: ['#3b4252', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#88c0d0', '#e5e9f0', '#4c566a', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#8fbcbb', '#eceff4'],
  },
  'solarized-dark': {
    background: '#002b36',
    foreground: '#839496',
    cursor: '#93a1a1',
    selectionBackground: 'rgba(7,54,66,0.95)',
    ansi: ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5', '#002b36', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
  },
  'solarized-light': {
    background: '#fdf6e3',
    foreground: '#657b83',
    cursor: '#586e75',
    selectionBackground: 'rgba(238,232,213,0.95)',
    ansi: ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5', '#002b36', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
  },
  'gruvbox-dark': {
    background: '#282828',
    foreground: '#ebdbb2',
    cursor: '#ebdbb2',
    selectionBackground: 'rgba(80,73,69,0.95)',
    ansi: ['#282828', '#cc241d', '#98971a', '#d79921', '#458588', '#b16286', '#689d6a', '#a89984', '#928374', '#fb4934', '#b8bb26', '#fabd2f', '#83a598', '#d3869b', '#8ec07c', '#ebdbb2'],
  },
  'tokyo-night': {
    background: '#1a1b26',
    foreground: '#c0caf5',
    cursor: '#c0caf5',
    selectionBackground: 'rgba(40,52,87,0.95)',
    ansi: ['#15161e', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#a9b1d6', '#414868', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#c0caf5'],
  },
  'catppuccin-mocha': {
    background: '#1e1e2e',
    foreground: '#cdd6f4',
    cursor: '#f5e0dc',
    selectionBackground: 'rgba(88,91,112,0.9)',
    ansi: ['#45475a', '#f38ba8', '#a6e3a1', '#f9e2af', '#89b4fa', '#f5c2e7', '#94e2d5', '#bac2de', '#585b70', '#f38ba8', '#a6e3a1', '#f9e2af', '#89b4fa', '#f5c2e7', '#94e2d5', '#a6adc8'],
  },
  'one-dark': {
    background: '#282c34',
    foreground: '#abb2bf',
    cursor: '#528bff',
    selectionBackground: 'rgba(62,68,81,0.95)',
    ansi: ['#282c34', '#e06c75', '#98c379', '#e5c07b', '#61afef', '#c678dd', '#56b6c2', '#abb2bf', '#5c6370', '#e06c75', '#98c379', '#e5c07b', '#61afef', '#c678dd', '#56b6c2', '#ffffff'],
  },
  'rose-pine': {
    background: '#191724',
    foreground: '#e0def4',
    cursor: '#524f67',
    selectionBackground: 'rgba(64,61,82,0.95)',
    ansi: ['#26233a', '#eb6f92', '#31748f', '#f6c177', '#9ccfd8', '#c4a7e7', '#ebbcba', '#e0def4', '#6e6a86', '#eb6f92', '#31748f', '#f6c177', '#9ccfd8', '#c4a7e7', '#ebbcba', '#e0def4'],
  },
  everforest: {
    background: '#2d353b',
    foreground: '#d3c6aa',
    cursor: '#d3c6aa',
    selectionBackground: 'rgba(71,82,88,0.95)',
    ansi: ['#475258', '#e67e80', '#a7c080', '#dbbc7f', '#7fbbb3', '#d699b6', '#83c092', '#d3c6aa', '#475258', '#e67e80', '#a7c080', '#dbbc7f', '#7fbbb3', '#d699b6', '#83c092', '#d3c6aa'],
  },
};

export const TERM_THEME_IDS = ['system', ...Object.keys(TERM_SCHEMES)];

/** The xterm-style theme object for a scheme id, or null for `system`/unknown. */
export function schemeTheme(id: string | null | undefined): Record<string, string> | null {
  const s = id ? TERM_SCHEMES[id] : undefined;
  if (!s) return null;
  const names = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
  const out: Record<string, string> = {
    background: s.background,
    foreground: s.foreground,
    cursor: s.cursor,
    selectionBackground: s.selectionBackground,
  };
  names.forEach((n, i) => {
    out[n] = s.ansi[i];
    out['bright' + n[0].toUpperCase() + n.slice(1)] = s.ansi[i + 8];
  });
  return out;
}

/** Ids of the user's own schemes (`~/.omniget/terminal/themes`), loaded by `loadUserSchemes`. */
const userIds = new Set<string>();
let userKey = '';

/** Every scheme id: built-in first, then the user's folder. */
export function termThemeIds(): string[] {
  return [...TERM_THEME_IDS, ...[...userIds].filter((id) => !TERM_THEME_IDS.includes(id))];
}

/**
 * Reads the user's Ghostty themes through Rust and merges them into
 * `TERM_SCHEMES` (a built-in id is never replaced). Returns true when the
 * set changed, so the renderer rebuilds its palettes.
 */
export async function loadUserSchemes(): Promise<boolean> {
  let list: Array<TermScheme & { id: string }>;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    list = await invoke<Array<TermScheme & { id: string }>>('term_themes');
  } catch {
    return false;
  }
  const key = JSON.stringify(list);
  if (key === userKey) return false;
  userKey = key;
  for (const id of userIds) delete TERM_SCHEMES[id];
  userIds.clear();
  for (const t of list) {
    if (TERM_THEME_IDS.includes(t.id)) continue;
    TERM_SCHEMES[t.id] = { background: t.background, foreground: t.foreground, cursor: t.cursor, selectionBackground: t.selectionBackground, ansi: t.ansi };
    userIds.add(t.id);
  }
  return true;
}
