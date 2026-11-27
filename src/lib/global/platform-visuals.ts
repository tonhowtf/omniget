export type GdTreeNode = {
  id: string;
  title: string;
  kind: string;
  thumbnail?: string | null;
  drm?: boolean;
  note?: string;
  children?: GdTreeNode[];
  source?: { format: string; url?: string };
};

export interface PlatformVisual {
  color: string;
  gradient: string;
  iconPaths: string;
}

const FALLBACK_COLORS = [
  "#FFAE70",
  "#0EA5E9",
  "#8B5CF6",
  "#10B981",
  "#EC4899",
  "#F59E0B",
  "#6366F1",
  "#14B8A6",
];

const DEFAULT_ICON_PATHS =
  '<path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" /><path d="M10 8l5 3-5 3V8z" />';

const PLATFORMS: Record<string, { color: string; iconPaths: string }> = {
  hotmart: {
    color: "#F04E23",
    iconPaths: '<path d="M7 4v16" /><path d="M17 4v16" /><path d="M7 12h10" /><circle cx="17" cy="5.5" r="1.5" fill="currentColor" stroke="none" />',
  },
  udemy: {
    color: "#A435F0",
    iconPaths: '<path d="M7 5l5-2.5L17 5" /><path d="M7 9.5v6a5 5 0 0 0 10 0v-6" />',
  },
  branas: {
    color: "#0EA5E9",
    iconPaths: '<path d="M7 5h6a3.5 3.5 0 0 1 0 7H7V5z" /><path d="M7 12h7a3.5 3.5 0 0 1 0 7H7v-7z" />',
  },
  "100tasks": {
    color: "#10B981",
    iconPaths: '<path d="M9 11l3 3 7-7" /><path d="M20 12v6a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h9" />',
  },
  entregadigital: {
    color: "#F59E0B",
    iconPaths: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z" /><path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" />',
  },
  fisicacomrenatobrito: {
    color: "#3B82F6",
    iconPaths: '<circle cx="12" cy="12" r="2.2" /><ellipse cx="12" cy="12" rx="9" ry="3.8" transform="rotate(30 12 12)" /><ellipse cx="12" cy="12" rx="9" ry="3.8" transform="rotate(-30 12 12)" />',
  },
  ggcheckout: {
    color: "#EC4899",
    iconPaths: '<path d="M6 7h12l-1 11H7L6 7z" /><path d="M9 7a3 3 0 0 1 6 0" /><path d="M10 12l2 2 3-3" />',
  },
  "tle-eliminators": {
    color: "#6366F1",
    iconPaths: '<polyline points="8 7 3 12 8 17" /><polyline points="16 7 21 12 16 17" /><line x1="14" y1="5" x2="10" y2="19" />',
  },
  courseflix: {
    color: "#E50914",
    iconPaths: '<rect x="3" y="5" width="18" height="14" rx="2.5" /><polygon points="10 9 16 12 10 15 10 9" fill="currentColor" stroke="none" />',
  },
  bunkr: {
    color: "#64748B",
    iconPaths: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />',
  },
  rocketseat: {
    color: "#8257E5",
    iconPaths: '<path d="M12 2l-4 6h8z" /><path d="M12 8v12" /><path d="M9 18l3 4 3-4" />',
  },
  teachable: {
    color: "#1FB5A3",
    iconPaths: '<path d="M6 6h12" /><path d="M12 6v14" />',
  },
  kajabi: {
    color: "#2E91FC",
    iconPaths: '<path d="M8 4v16" /><path d="M8 12l6-8" /><path d="M8 12c2 2 4 5 7 7" />',
  },
  skool: {
    color: "#FBBF24",
    iconPaths: '<path d="M17 7a5 3 0 0 0-10 0c0 2 2 3 5 5s5 1 5 3a5 3 0 0 1-10 0" />',
  },
  gumroad: {
    color: "#FF90E8",
    iconPaths: '<circle cx="12" cy="12" r="8" /><path d="M12 8a4 4 0 1 0 0 8v-4h3" />',
  },
};

function hashIndex(seed: string, length: number): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % length;
}

export function platformVisual(idOrName: string | null | undefined): PlatformVisual {
  const key = (idOrName ?? "").trim().toLowerCase();
  const known = PLATFORMS[key];
  const color = known?.color ?? FALLBACK_COLORS[hashIndex(key || "course", FALLBACK_COLORS.length)];
  const iconPaths = known?.iconPaths ?? DEFAULT_ICON_PATHS;
  const gradient = `radial-gradient(120% 90% at 15% 10%, color-mix(in srgb, ${color} 34%, transparent), transparent 65%), linear-gradient(135deg, color-mix(in srgb, ${color} 18%, var(--surface-hi, var(--surface))) 0%, var(--surface) 100%)`;
  return { color, gradient, iconPaths };
}

export function collectDownloadable(node: GdTreeNode): GdTreeNode[] {
  if (!node.children?.length) {
    return node.source && !node.drm ? [node] : [];
  }
  return node.children.flatMap(collectDownloadable);
}

export function countDrm(node: GdTreeNode): number {
  if (!node.children?.length) return node.drm ? 1 : 0;
  return node.children.reduce((acc, child) => acc + countDrm(child), 0);
}

export function countModules(node: GdTreeNode): number {
  return (node.children ?? []).filter((c) => !!c.children?.length).length;
}

/**
 * Returns true when a resolved tree represents a library of courses (multiple
 * top-level folders with downloadable items) rather than a single course or page.
 */
export function isCourseLibrary(tree: GdTreeNode): boolean {
  const children = tree.children ?? [];
  if (children.length < 2) return false;
  const foldersWithItems = children.filter(
    (c) => !!c.children?.length && (collectDownloadable(c).length > 0 || countDrm(c) > 0),
  );
  if (foldersWithItems.length < 2) return false;
  // If top-level folders themselves contain subfolders (modules) OR have thumbnails,
  // it is definitely a multi-course library. Otherwise treat as a single course with modules.
  return foldersWithItems.some((c) => Boolean(c.thumbnail) || countModules(c) > 0);
}

export function findThumbnail(node: GdTreeNode): string | null {
  if (node.thumbnail) return node.thumbnail;
  for (const child of node.children ?? []) {
    const nested = findThumbnail(child);
    if (nested) return nested;
  }
  return null;
}
