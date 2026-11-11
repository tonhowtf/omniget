const actions = new WeakMap<Element, readonly string[]>();
export function agentActions(node: HTMLElement, allowed: readonly string[]) {
  actions.set(node, allowed);
  return { destroy() { actions.delete(node); } };
}
const CONTROLS = 'button,a[href],summary,label,input,textarea,select,[role=button],[role=link],[role=tab],[role=checkbox],[role=switch],[role=radio],[role=option],[role=menuitem]';
const SCROLLERS = '[data-agent-region],[role=list],[role=listbox],[role=grid],[role=feed]';
export function allowedActions(node: Element, unguarded = false): readonly string[] {
  const own = actions.get(node);
  if (own) return own;
  if (!unguarded && !import.meta.env.DEV) return [];
  if (node.matches(CONTROLS)) return ['click', 'fill', 'scroll'];
  return unguarded && node.matches(SCROLLERS) ? ['scroll'] : [];
}
