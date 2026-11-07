export function focusPayload(path: string): { path: string; jobId: null; conversationId: null } {
  return { path: path.replace(/\/+$/, "") || "/", jobId: null, conversationId: null };
}
