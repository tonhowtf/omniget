// First-open renderer calibration. The house used to measure the GPU the
// first time the World opened and keep the result in settings
// (`world.tier_measured`, per app version); the city reads that value and
// falls back to tier 1 without it. The canvas is now what opens first, so it
// takes over: a pinned tier wins, a measurement of this app version is kept,
// otherwise it measures once, off the critical path.

import { getSettings, updateSettings } from '$lib/stores/settings-store.svelte';

function isTier(v: unknown): v is 0 | 1 | 2 | 3 {
  return v === 0 || v === 1 || v === 2 || v === 3;
}

let running: Promise<void> | null = null;

/** Measures and persists the render tier when settings do not have a fresh one. Never throws. */
export function ensureRenderTier(): Promise<void> {
  if (!running) running = run().catch(() => {});
  return running;
}

async function run(): Promise<void> {
  const world = getSettings()?.world;
  if (isTier(world?.tier_override)) return;
  let appVersion = '';
  try {
    appVersion = await (await import('@tauri-apps/api/app')).getVersion();
  } catch {
    appVersion = '';
  }
  if (isTier(world?.tier_measured) && appVersion && world?.measured_app_version === appVersion) return;
  const { calibrateDetailed } = await import('$lib/gfx/calibrate');
  const cal = await calibrateDetailed(null);
  await updateSettings({
    world: {
      tier_measured: cal.tier,
      measured_median_ms: Number.isFinite(cal.medianMs) ? cal.medianMs : null,
      measured_app_version: appVersion || null,
    },
  });
}
