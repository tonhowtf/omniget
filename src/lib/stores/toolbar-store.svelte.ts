type ToolbarSegment = {
  id: string;
  label: string;
  count?: number;
};

export type ToolbarAction = {
  id: string;
  label: string;
  icon?: string;
  iconFilled?: boolean;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  prominent?: boolean;
  showLabel?: boolean;
};

type ToolbarState = {
  segments?: ToolbarSegment[];
  activeSegment?: string;
  onSegment?: (id: string) => void;
  actions?: ToolbarAction[];
};

let state = $state<ToolbarState>({});
let owner: symbol | null = null;

export function getToolbar(): ToolbarState {
  return state;
}

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
