export function toBody(node: HTMLElement): { destroy(): void } {
  if (typeof document !== 'undefined') document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
