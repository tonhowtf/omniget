import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import * as api from "$lib/video/api";
import { openPlayer } from "$lib/video/player.svelte";
import { go } from "$lib/video/ui.svelte";
import type { MediaItem, RootKind } from "$lib/video/types";
import type { MenuItem } from "#components/music/ContextMenu.svelte";
import { errText } from "./util";

const tr = (key: string, vars?: Record<string, unknown>) => get(t)(key, vars) as string;

export async function addFolder(kind: RootKind): Promise<boolean> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ directory: true, multiple: false }).catch(() => null);
  if (!picked || Array.isArray(picked)) return false;
  try {
    if (kind === "course") {
      const c = await api.courseImport(picked);
      showToast("success", tr("video.toast.course_imported", { name: c.title }));
    } else {
      await api.rootsAdd(picked, "library");
      await api.scan();
      showToast("info", tr("video.toast.folder_added"));
    }
    return true;
  } catch (e) {
    showToast("error", errText(e));
    return false;
  }
}

export async function markWatched(keys: string[], watched: boolean, after?: () => void) {
  try {
    await api.setWatched(keys, watched);
    after?.();
  } catch (e) {
    showToast("error", errText(e));
  }
}

export function itemMenu(it: MediaItem, reload: () => void): MenuItem[] {
  const watched = !!it.progress?.watched;
  const menu: MenuItem[] = [
    { label: tr("video.menu.play"), icon: "play", action: () => void openPlayer({ kind: "local", id: it.id }) },
    { label: tr("video.menu.play_from_start"), icon: "repeat", action: () => void openPlayer({ kind: "local", id: it.id }, { startMs: 0 }) },
    { divider: true },
    watched
      ? { label: tr("video.menu.mark_unwatched"), icon: "close", action: () => void markWatched([it.key], false, reload) }
      : { label: tr("video.menu.mark_watched"), icon: "check", action: () => void markWatched([it.key], true, reload) },
  ];
  if (it.show) menu.push({ label: tr("video.menu.go_to_show"), icon: "playlist", action: () => go({ name: "show", show: it.show! }) });
  if (it.course_id != null) menu.push({ label: tr("video.menu.go_to_course"), icon: "playlist", action: () => go({ name: "course", id: it.course_id! }) });
  menu.push(
    { divider: true },
    { label: tr("video.menu.reveal"), icon: "folder", action: () => void api.reveal(it.id).catch((e) => showToast("error", errText(e))) },
    { label: tr("video.menu.open_external"), icon: "external", action: () => void api.openExternal({ kind: "local", id: it.id }).catch((e) => showToast("error", errText(e))) },
  );
  return menu;
}
