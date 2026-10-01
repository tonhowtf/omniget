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
  newWorkspaceAsk = $state(0);
  newFloorAsk = $state(0);

  toggleSidebar(): void {
    this.sidebarOpen = !this.sidebarOpen;
    try {
      localStorage.setItem(SIDEBAR_KEY, this.sidebarOpen ? '1' : '0');
    } catch {
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
