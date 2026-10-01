// Role icons: a named stroke icon (24×24) or, for anything else, the string
// itself (an emoji). Names match the built-ins of core::canvas::roles.

const ROLE_ICONS: Record<string, string> = {
  crown: 'M3 8l4 4 5-7 5 7 4-4-2 11H5z',
  hammer: 'M14 6l4 4 M4 20l9-9 M11 4l6 6 3-3-6-6z',
  eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  flask: 'M9 3h6 M10 3v6L4 19a1.5 1.5 0 0 0 1.3 2h13.4a1.5 1.5 0 0 0 1.3-2L14 9V3 M7 14h10',
  sparkles: 'M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z',
  bug: 'M8 8a4 4 0 0 1 8 0v8a4 4 0 0 1-8 0z M12 8v12 M4 12h4 M16 12h4 M5 7l3 2 M19 7l-3 2 M5 18l3-2 M19 18l-3-2',
  music: 'M9 18V5l11-2v13 M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0z M20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z',
  rocket: 'M5 15c-1.5 1.5-2 5-2 5s3.5-.5 5-2 M9 15l-3-3c1-4 5-9 13-9 0 8-5 12-9 13z M14 10a1.5 1.5 0 1 0 0-.01',
  shuffle: 'M16 3h5v5 M4 20L21 3 M21 16v5h-5 M15 15l6 6 M4 4l5 5',
  book: 'M4 19V5a2 2 0 0 1 2-2h14v16H6a2 2 0 0 0-2 2 2 2 0 0 0 2 2h14',
  'git-branch': 'M6 3v12 M18 9a3 3 0 1 0 0-.01 M6 21a3 3 0 1 0 0-.01 M18 9a9 9 0 0 1-9 9',
  gauge: 'M12 14l4-4 M4 18a9 9 0 1 1 16 0z',
  user: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M4 21a8 8 0 0 1 16 0',
  star: 'M12 3l2.8 5.7 6.2.9-4.5 4.4 1 6.2L12 17.3 6.5 20.2l1-6.2L3 9.6l6.2-.9z',
  shield: 'M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z',
  terminal: 'M4 5h16v14H4z M7 9l3 3-3 3 M12 15h5',
};

export const ICON_NAMES = Object.keys(ROLE_ICONS);

/** Palette offered by the editor (the built-ins use these too). */
export const ROLE_COLORS = [
  '#D97757',
  '#EF4444',
  '#F97316',
  '#EAB308',
  '#22C55E',
  '#10A37F',
  '#06B6D4',
  '#3B82F6',
  '#8B5CF6',
  '#A855F7',
  '#EC4899',
  '#94A3B8',
];

export function iconPath(icon: string | undefined): string | null {
  if (!icon) return null;
  return ROLE_ICONS[icon] ?? null;
}
