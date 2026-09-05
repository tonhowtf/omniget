/**
 * Toolbar actions published by the current page.
 *
 * macOS toolbars act on the content beneath them, so a page (Downloads,
 * Marketplace, …) registers its segmented control and trailing buttons here
 * and the shell renders them in the titlebar. Pages call `setToolbar` inside
 * an `$effect` and return the cleanup so the toolbar empties on navigation.
 */

export type ToolbarSegment = {
  id: string;
  label: string;
  count?: number;
};

export type ToolbarAction = {
  id: string;
  label: string;
  /** Path data for a 16×16 SF-style symbol drawn with `currentColor` stroke. */
  icon?: string;
  /** Filled icon (uses `fill` instead of `stroke`). */
  iconFilled?: boolean;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  prominent?: boolean;
  /** Show the label next to the icon (default: icon-only with tooltip). */
  showLabel?: boolean;
};

export type ToolbarState = {
  segments?: ToolbarSegment[];
  activeSegment?: string;
  onSegment?: (id: string) => void;
  actions?: ToolbarAction[];
};

let state = $state<ToolbarState>({});

export function getToolbar(): ToolbarState {
  return state;
}

/** Replace the toolbar contents. Returns a cleanup that clears them. */
export function setToolbar(next: ToolbarState): () => void {
  state = next;
  return () => {
    if (state === next) state = {};
  };
}

export function clearToolbar(): void {
  state = {};
}
