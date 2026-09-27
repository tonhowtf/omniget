# OmniGet's own Windows tweaks

- Repository: https://github.com/tonhowtf/omniget, license GPL-3.0 (see `LICENSE`). Each item says its own origin in
  `source`/`license`/`upstream`.
- `ours.json`, written by hand:
  - memory integrity (Core Isolation / HVCI) off, as the only `extreme` item, with the warning that it trades
    security for FPS; screen only, never in an agent plan;
  - power plans Balanced / High performance / Ultimate (winutil `Invoke-WPFUltimatePerformance.ps1`, MIT) as one
    group; undo goes back to the plan that was active and deletes the copy we created;
  - "Delay updates (security first)": the registry part of winutil's `Invoke-WPFUpdatessecurity.ps1` (MIT), Pro and
    higher editions only; Windows Update is never turned off;
  - the old `win_tweaks.rs` (Sophia ideas and hardentools, GPL-3.0): clock seconds, Widgets button, AutoRun,
    AutoPlay, Windows Script Host, Office macros and DDE, Acrobat JavaScript, UAC prompt, safe DLL search. Its
    privacy items moved to the privacy module; Defender PUA and LSA protection were dropped (both touch keys
    washmybelly never changes).
