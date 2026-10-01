// Account + local subscriptions for the video screens. Backed by the shell's
// session (W5), so subscribing here updates the guide's channel list too.
import type { LocalSub } from "$lib/tube/types";
import { loadSession, session as shell, signIn } from "../shell/session.svelte";

export function ensureSession(): Promise<void> {
  return loadSession();
}

export function sessionReady(): boolean {
  return shell.loaded;
}

export function loggedIn(): boolean {
  return !!shell.account?.logged_in;
}

export function account() {
  return shell.account;
}

export function localSub(channelId: string): LocalSub | null {
  return shell.subs.find((s) => s.channel_id === channelId) ?? null;
}

export function localSubCount(): number {
  return shell.subs.length;
}

export function putLocalSub(sub: LocalSub) {
  shell.subs = [...shell.subs.filter((s) => s.channel_id !== sub.channel_id), sub];
}

export function dropLocalSub(channelId: string) {
  shell.subs = shell.subs.filter((s) => s.channel_id !== channelId);
}

/** Sign in through the shell (refreshes the session afterwards). */
export async function login(): Promise<void> {
  await signIn();
}
