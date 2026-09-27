# winutil tweaks (Windows tweaks)

- Upstream: https://github.com/ChrisTitusTech/winutil, license MIT (see `LICENSE` here, Copyright (c) 2022 CT Tech Group LLC).
- Commit: `ad81e0a7aff424932afff77504e7440e07e19a15` (2026-09-26). Read: 2026-09-27.
- Converted: `config/tweaks.json` (not strict JSON: raw line breaks inside strings, escaped before parsing) →
  `tweaks.json` (21 items). Only a curated list (`INCLUDE_WINUTIL` in the converter), and only the `registry[]` part
  as typed ops; `OriginalValue` becomes "Back to the Windows default", `DefaultState` becomes `absent_means`.
  `<RemoveEntry>` becomes a `reg_delete`. The Multiplane Overlay combobox becomes a group of three.
- Never taken: every `InvokeScript`/`UndoScript` (PowerShell), the "Services - Set to Manual" list (breaks the
  hotspot, Bluetooth, printing), BitLocker - Disable, Edge removal, the restore point tweak (lifts Windows' 24 h
  limit), the Adobe hosts list (downloaded at run time), O&O ShutUp10++, Razer/Logitech blocks, the RDP warning
  switch (weakens security), the Start menu feature override, disk cleanup with `/ResetBase`, and entries that
  duplicate a Win11Debloat item. The full list is in `tweaks.json` → `excluded`.
- The update deferral ("Security" mode) and Ultimate Performance items are winutil PowerShell functions
  re-expressed by hand as typed ops in `../tweaks/ours.json`, with winutil as their source.
