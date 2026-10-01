/**
 * Toolbar actions published by the current page.
 *
 * macOS toolbars act on the content beneath them, so a page (Downloads, …)
 * registers its segmented control and trailing buttons here
 * and the shell renders them in the titlebar. Pages call `setToolbar` inside
 * an `$effect` and return the cleanup so the toolbar empties on navigation.
 */

type ToolbarSegment = {
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

type ToolbarState = {
  segments?: ToolbarSegment[];
  activeSegment?: string;
  onSegment?: (id: string) => void;
  actions?: ToolbarAction[];
};

let state = $state<ToolbarState>({});
// Registration identity must not depend on the proxy created by $state.
let owner: symbol | null = null;

export function getToolbar(): ToolbarState {
  return state;
}

/** Replace the toolbar contents. Returns a cleanup that clears them. */
export function setToolbar(next: ToolbarState): () => void {
  const registration = Symbol("toolbar");
  owner = registration;
  state = next;
  return () => {
    if (owner === registration) clearToolbar();
  };
}

function clearToolbar(): void {
  owner = null;
  state = {};
}
