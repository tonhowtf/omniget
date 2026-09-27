/** On-demand canvas adapters. Register only while the real renderer is mounted. */
const adapters = new Map<string, () => unknown>();
export function registerSemantic(id: string, read: () => unknown): () => void {
  adapters.set(id, read);
  return () => { if (adapters.get(id) === read) adapters.delete(id); };
}
export function readSemantics(): Record<string, unknown> {
  return Object.fromEntries([...adapters].map(([id, read]) => [id, read()]));
}
