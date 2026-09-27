# Win11Debloat features (Windows tweaks)

- Upstream: https://github.com/Raphire/Win11Debloat, license MIT (see `LICENSE` here, Copyright (c) 2020 Raphire).
- Commit: `32024662f3c602442e7af82bbf52c89143b31aeb` (2026-09-10). Read: 2026-09-27.
- Converted: `Config/Features.json` → `features.json` (86 tweaks, 9 exclusive groups from `UiGroups`).
- `Regfiles/` and `Regfiles/Undo/`: the 140 `.reg` files those tweaks use, copied byte for byte (UTF-16 LE with BOM or
  UTF-8, as upstream ships them). The Rust side (`tweaks/regfile.rs`) parses them into typed ops; the Undo file is
  "Back to the Windows default". Nothing is run as PowerShell.
- Left out (listed in `features.json` → `excluded`): telemetry, location and Find My Device (the privacy module owns
  them), BitLocker auto-encryption (BitLocker stays out of washmybelly), Delivery Optimization (writes the
  NetworkService hive; the winutil policy is used instead), and the features upstream implements in PowerShell
  (ClearStart, Store search suggestions, Widgets removal). Windows Sandbox and WSL map to the typed `optional_feature`
  op, which waits for a helper update.
- Converter: `node scripts/washmybelly/tweaks-convert.mjs --repos <folder>`; `--check` fails when anything here or
  `tweaks/regfiles.rs` is out of date.
