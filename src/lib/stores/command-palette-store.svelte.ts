export type CommandPaletteItem = {
  id: string;
  label: string;
  group: string;
  keywords?: string;
  action: () => void;
};

let open = $state(false);
let query = $state("");
let selectedIndex = $state(0);
let items = $state<CommandPaletteItem[]>([]);

export function openCommandPalette() {
  open = true;
  query = "";
  selectedIndex = 0;
}

export function closeCommandPalette() {
  open = false;
  query = "";
  selectedIndex = 0;
}

export function isCommandPaletteOpen(): boolean {
  return open;
}

export function getCommandPaletteQuery(): string {
  return query;
}

export function setCommandPaletteQuery(value: string) {
  query = value;
  selectedIndex = 0;
}

export function getCommandPaletteItems(): CommandPaletteItem[] {
  return items;
}

export function setCommandPaletteItems(next: CommandPaletteItem[]) {
  items = next;
}

export function getCommandPaletteSelectedIndex(): number {
  return selectedIndex;
}

export function setCommandPaletteSelectedIndex(index: number) {
  selectedIndex = index;
}

export function moveCommandPaletteSelection(delta: number, filteredCount: number) {
  if (filteredCount <= 0) return;
  selectedIndex = (selectedIndex + delta + filteredCount) % filteredCount;
}

export function runCommandPaletteSelected(filtered: CommandPaletteItem[]) {
  const item = filtered[selectedIndex];
  if (!item) return;
  closeCommandPalette();
  item.action();
}
