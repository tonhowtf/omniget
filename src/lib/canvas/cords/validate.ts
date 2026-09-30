// Which nodes a cord may join (Maestri, estudo 100 M-F1): terminal↔terminal,
// terminal↔note, note↔note, terminal↔portal and portal↔portal. A note (or a
// fichário) never connects to a portal, and a node never to itself. Mirror of
// Rust `canvas::cords::cord_allowed`.

type Ends = { id: string; kind: string } | null | undefined;

/** `null` when the cord is allowed, else the i18n key of the refusal. */
export function cordRefusal(a: Ends, b: Ends): string | null {
  if (!a || !b) return null;
  if (a.id === b.id) return 'canvas.engine.cordInvalid';
  const notey = (k: string) => k === 'note' || k === 'stack';
  if ((notey(a.kind) && b.kind === 'portal') || (notey(b.kind) && a.kind === 'portal')) {
    return 'canvas.engine.cordInvalid';
  }
  return null;
}
