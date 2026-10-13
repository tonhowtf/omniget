const actions = new WeakMap<Element, readonly string[]>();
export function agentActions(node: HTMLElement, allowed: readonly string[]) {
  actions.set(node, allowed);
  return { destroy() { actions.delete(node); } };
}
export function allowedActions(node: Element): readonly string[] {
  return actions.get(node) ?? (import.meta.env.DEV && node.matches('button,a[href],[role=tab],[role=checkbox],input,[role=option],[role=menuitem]') ? ['click','fill'] : []);
}
