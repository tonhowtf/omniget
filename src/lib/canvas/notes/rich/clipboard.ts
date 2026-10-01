async function pluginText(text: string): Promise<boolean> {
  try {
    const m = await import('@tauri-apps/plugin-clipboard-manager');
    await m.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return pluginText(text);
  }
}

export async function copyFlavours(flavours: Record<string, string>, plain: string): Promise<boolean> {
  const CI = (globalThis as { ClipboardItem?: typeof ClipboardItem }).ClipboardItem;
  if (CI && navigator.clipboard?.write) {
    const tries = [flavours, Object.fromEntries(Object.entries(flavours).filter(([k]) => k === 'text/html' || k === 'text/plain'))];
    for (const f of tries) {
      try {
        const item: Record<string, Blob> = {};
        for (const [type, value] of Object.entries(f)) item[type] = new Blob([value], { type });
        if (!item['text/plain']) item['text/plain'] = new Blob([plain], { type: 'text/plain' });
        await navigator.clipboard.write([new CI(item)]);
        return true;
      } catch {
      }
    }
  }
  return copyText(plain);
}

export async function copyPng(png: Blob | Promise<Blob>, fallbackText: string): Promise<boolean> {
  const CI = (globalThis as { ClipboardItem?: typeof ClipboardItem }).ClipboardItem;
  if (CI && navigator.clipboard?.write) {
    try {
      await navigator.clipboard.write([new CI({ 'image/png': png })]);
      return true;
    } catch {
    }
  }
  return copyText(fallbackText);
}

export async function svgToPng(svg: string, width: number, height: number, background: string): Promise<Blob> {
  const scale = 2;
  const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml;charset=utf-8' }));
  try {
    const img = new Image();
    img.decoding = 'sync';
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error('svg decode failed'));
      img.src = url;
    });
    const c = document.createElement('canvas');
    c.width = Math.ceil(width * scale);
    c.height = Math.ceil(height * scale);
    const g = c.getContext('2d');
    if (!g) throw new Error('no 2d context');
    g.fillStyle = background || '#ffffff';
    g.fillRect(0, 0, c.width, c.height);
    g.drawImage(img, 0, 0, c.width, c.height);
    return await new Promise<Blob>((resolve, reject) => c.toBlob((b) => (b ? resolve(b) : reject(new Error('toBlob failed'))), 'image/png'));
  } finally {
    URL.revokeObjectURL(url);
  }
}
