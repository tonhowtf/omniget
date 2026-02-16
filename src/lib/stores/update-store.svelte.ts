import { checkForUpdate, type UpdateInfo } from "$lib/updater";

let updateInfo: UpdateInfo = $state({ available: false });

export function getUpdateInfo(): UpdateInfo {
  return updateInfo;
}

export async function refreshUpdateInfo(): Promise<void> {
  updateInfo = await checkForUpdate();
}
