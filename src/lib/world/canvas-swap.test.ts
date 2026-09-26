// The GL wake fallback swaps the <canvas> for a fresh one. Input must move to
// the new element and nothing may stay attached to the old one.
import { afterEach, describe, expect, it } from 'vitest';
import { followCanvas } from './input';
import { replaceCanvas } from './render/gl2';
import { CANVAS_REPLACED_EVENT } from './render/types';

/** Just enough of an element for replaceCanvas: attributes, size, a parent. */
class FakeCanvas extends EventTarget {
  width = 0;
  height = 0;
  parentNode: FakeParent | null = null;
  private attrs = new Map<string, string>();
  listeners = 0;
  override addEventListener(t: string, l: EventListenerOrEventListenerObject | null, o?: boolean | AddEventListenerOptions) {
    this.listeners++;
    super.addEventListener(t, l, o);
  }
  override removeEventListener(t: string, l: EventListenerOrEventListenerObject | null, o?: boolean | EventListenerOptions) {
    this.listeners--;
    super.removeEventListener(t, l, o);
  }
  setAttribute(n: string, v: string) {
    this.attrs.set(n, v);
  }
  getAttribute(n: string) {
    return this.attrs.get(n) ?? null;
  }
  get attributes() {
    return [...this.attrs].map(([name, value]) => ({ name, value }));
  }
}
class FakeParent {
  children: FakeCanvas[] = [];
  replaceChild(next: FakeCanvas, old: FakeCanvas) {
    this.children[this.children.indexOf(old)] = next;
    next.parentNode = this;
    old.parentNode = null;
  }
}

const g = globalThis as unknown as { document?: unknown };
const hadDocument = 'document' in g;
function withDocument() {
  g.document = { createElement: () => new FakeCanvas() };
}
afterEach(() => {
  if (!hadDocument) delete g.document;
});

function mount(): { parent: FakeParent; canvas: FakeCanvas } {
  const parent = new FakeParent();
  const canvas = new FakeCanvas();
  canvas.width = 1632;
  canvas.height = 918;
  canvas.setAttribute('class', 'canvas s-abc');
  canvas.setAttribute('tabindex', '0');
  canvas.setAttribute('aria-label', 'The city');
  parent.children.push(canvas);
  canvas.parentNode = parent;
  return { parent, canvas };
}

describe('canvas swap on wake', () => {
  it('replaceCanvas keeps every attribute and size, and announces the new element', () => {
    withDocument();
    const { parent, canvas } = mount();
    let announced: unknown = null;
    canvas.addEventListener(CANVAS_REPLACED_EVENT, (e) => (announced = (e as CustomEvent).detail));
    const next = replaceCanvas(canvas as unknown as HTMLCanvasElement) as unknown as FakeCanvas;
    expect(next).not.toBe(canvas);
    expect(parent.children[0]).toBe(next);
    expect(announced).toBe(next);
    expect([next.getAttribute('tabindex'), next.getAttribute('aria-label'), next.getAttribute('class')]).toEqual(['0', 'The city', 'canvas s-abc']);
    expect([next.width, next.height]).toEqual([1632, 918]);
  });

  it('input follows the swap: the new element gets clicks, the old one keeps nothing', () => {
    withDocument();
    const { canvas } = mount();
    const clicks: string[] = [];
    const seen: unknown[] = [];
    const bind = (el: FakeCanvas) => {
      const on = () => clicks.push(el.getAttribute('data-name') ?? '?');
      el.addEventListener('pointerdown', on);
      return () => el.removeEventListener('pointerdown', on);
    };
    canvas.setAttribute('data-name', 'old');
    const baseline = canvas.listeners;
    const detach = followCanvas(canvas, bind, (next) => seen.push(next));
    expect(canvas.listeners).toBe(baseline + 2); // input + swap watcher

    const next = replaceCanvas(canvas as unknown as HTMLCanvasElement) as unknown as FakeCanvas;
    next.setAttribute('data-name', 'new');
    expect(seen).toEqual([next]);
    // Old element: every listener this binding added is gone.
    expect(canvas.listeners).toBe(baseline);
    canvas.dispatchEvent(new Event('pointerdown'));
    next.dispatchEvent(new Event('pointerdown'));
    expect(clicks).toEqual(['new']);

    // A second swap works the same way, and detaching leaves nothing behind.
    const third = replaceCanvas(next as unknown as HTMLCanvasElement) as unknown as FakeCanvas;
    third.setAttribute('data-name', 'third');
    third.dispatchEvent(new Event('pointerdown'));
    next.dispatchEvent(new Event('pointerdown'));
    expect(clicks).toEqual(['new', 'third']);
    detach();
    expect(third.listeners).toBe(0);
    third.dispatchEvent(new Event('pointerdown'));
    expect(clicks).toEqual(['new', 'third']);
  });
});
