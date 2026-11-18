const GLOBALS = ["document", "DOMParser", "XMLSerializer", "NodeFilter"] as const;

export async function installDom(): Promise<boolean> {
  if (typeof DOMParser !== "undefined" && typeof document !== "undefined") return true;
  for (const name of ["happy-dom", "jsdom"]) {
    try {
      const mod = await import(/* @vite-ignore */ name);
      const win = mod.Window ? new mod.Window() : new mod.JSDOM("").window;
      for (const key of GLOBALS) (globalThis as Record<string, unknown>)[key] = win[key];
      return true;
    } catch {
      continue;
    }
  }
  return false;
}
