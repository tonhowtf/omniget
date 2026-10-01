const actions = new WeakMap<Element, readonly string[]>();
export function agentActions(node: HTMLElement, allowed: readonly string[]) {
  actions.set(node, allowed);
  return { destroy() { actions.delete(node); } };
}
export function allowedActions(node: Element): readonly string[] {
  return actions.get(node) ?? [];
}
