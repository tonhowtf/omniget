type Ends = { id: string; kind: string } | null | undefined;

export function cordRefusal(a: Ends, b: Ends): string | null {
  if (!a || !b) return null;
  if (a.id === b.id) return 'canvas.engine.cordInvalid';
  const notey = (k: string) => k === 'note' || k === 'stack';
  if ((notey(a.kind) && b.kind === 'portal') || (notey(b.kind) && a.kind === 'portal')) {
    return 'canvas.engine.cordInvalid';
  }
  return null;
}
