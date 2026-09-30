// Small shared UI state between the Batuta palette and the workspace sidebar
// (both mounted inside Canvas.svelte), so each can ask the other for things.

const SIDEBAR_KEY = 'omniget.canvas.sidebarOpen';

function readSidebar(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_KEY) !== '0';
  } catch {
    return true;
  }
}

class CanvasUi {
  sidebarOpen = $state(readSidebar());
  /** Bumped to ask the sidebar to open its "new workspace" form. */
  newWorkspaceAsk = $state(0);
  /** Bumped to ask the sidebar to add a floor to the current workspace. */
  newFloorAsk = $state(0);

  toggleSidebar(): void {
    this.sidebarOpen = !this.sidebarOpen;
    try {
      localStorage.setItem(SIDEBAR_KEY, this.sidebarOpen ? '1' : '0');
    } catch {
      // Private mode.
    }
  }

  askNewWorkspace(): void {
    if (!this.sidebarOpen) this.toggleSidebar();
    this.newWorkspaceAsk++;
  }

  askNewFloor(): void {
    if (!this.sidebarOpen) this.toggleSidebar();
    this.newFloorAsk++;
  }
}

export const canvasUi = new CanvasUi();
