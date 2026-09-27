# Sophia Script tweaks (Windows tweaks)

- Upstream: https://github.com/farag2/Sophia-Script-for-Windows, license MIT (see `LICENSE` here, Copyright (c) farag2).
- Commit: `1b761b48e3f680b6669c37ad61d92518d583a44e` (2026-09-24). Read: 2026-09-27.
- `tweaks.json`: 10 items extracted by hand from `src/Sophia_Script_for_Windows_11/Module/Sophia.psm1` (the function
  name is in `upstream`): GPUScheduling (HAGS), XboxGameTips, PrtScnSnippingTool, AeroShaking, JPEGWallpapersQuality,
  RestartNotification, ActiveHours, SaveRestartableApps, NavigationPaneExpand, RecentlyAddedApps. Each keeps both
  parameter sets: the one we apply and the opposite one as "Back to the Windows default".
- HAGS is offered only with a dedicated GPU, outside a virtual machine and with WDDM 2.7+ (SophiApp `GPUScheduling`,
  MIT; our GPU test reads adapter names instead of SophiApp's `AdapterDACType` check).
- Not taken: the DiagTrack firewall block (`DiagTrackService`): it needs a firewall op the helper does not have, and
  telemetry belongs to the privacy module.
