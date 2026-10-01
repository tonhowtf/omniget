import { chats, startChatEvents } from './store.svelte';

interface ChatHost {
  focused(): string | null;
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
