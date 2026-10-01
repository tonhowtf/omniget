// Chat face registration. Canvas.svelte (not owned here) calls
// `registerChat()` on mount and the returned function on destroy:
//   - starts the `canvas://chat` listener;
//   - ⌘J on a focused terminal node toggles its chat face.
// TerminalNode shows <ChatFace> instead of the terminal body when
// `chats.isChat(node.id)`.

import { chats, startChatEvents } from './store.svelte';

interface ChatHost {
  /** Focused node id, or null. */
  focused(): string | null;
  /** Whether a node id is a terminal/agent node. */
  isTerminal(id: string): boolean;
}

export function registerChat(host: ChatHost): () => void {
  const stop = startChatEvents();
  const onKey = (e: KeyboardEvent) => {
    if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== 'j') return;
    const id = host.focused();
    if (!id || !host.isTerminal(id)) return;
    e.preventDefault();
    e.stopPropagation();
    chats.toggle(id);
  };
  window.addEventListener('keydown', onKey, true);
  return () => {
    window.removeEventListener('keydown', onKey, true);
    stop();
  };
}
