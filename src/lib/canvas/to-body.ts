// Svelte action: moves a modal to <body> so it escapes the stacking context
// of the chrome it was declared in (the floor bar sits in the z-indexed
// bottom-right corner, under the toolbar and the workspace sidebar).
export function toBody(node: HTMLElement): { destroy(): void } {
  if (typeof document !== 'undefined') document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
