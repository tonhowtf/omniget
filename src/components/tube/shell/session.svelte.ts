import * as api from "$lib/tube/api";
import type { Account, LocalSub, PlaylistItem } from "$lib/tube/types";

export const session = $state({
  loaded: false,
  account: null as Account | null,
  subs: [] as LocalSub[],
  musicPlaylists: [] as PlaylistItem[],
  busy: false,
});

let loading: Promise<void> | null = null;

export function loadSession(force = false): Promise<void> {
  if (loading && !force) return loading;
  loading = (async () => {
    const [st, subs] = await Promise.allSettled([api.status(), api.subsList()]);
    session.account = st.status === "fulfilled" ? st.value.account : null;
    session.subs = subs.status === "fulfilled" ? subs.value : [];
    session.loaded = true;
    if (session.account?.logged_in) {
      try {
        const p = await api.musicLibrary("playlists");
        session.musicPlaylists = p.shelves.flatMap((s) => s.items).filter((i): i is PlaylistItem => i.kind === "playlist");
      } catch {
        session.musicPlaylists = [];
      }
    } else session.musicPlaylists = [];
  })();
  return loading;
}

export async function signIn(): Promise<void> {
  session.busy = true;
  try {
    session.account = await api.login();
    await loadSession(true);
  } finally {
    session.busy = false;
  }
}

export async function signOut(): Promise<void> {
  session.busy = true;
  try {
    await api.logout();
    await loadSession(true);
  } finally {
    session.busy = false;
  }
}
