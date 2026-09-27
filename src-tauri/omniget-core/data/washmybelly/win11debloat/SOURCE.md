# Win11Debloat app list (Windows)

- Upstream: https://github.com/Raphire/Win11Debloat, license MIT (see `LICENSE` here, Copyright (c) 2020 Raphire).
- Commit: `32024662f3c602442e7af82bbf52c89143b31aeb` (2026-09-10).
- Read: 2026-09-27.
- File converted: `Config/Apps.json` (141 apps). `Recommendation` `unsafe` is written as `dangerous`; `RemovalMethod` becomes `removal` (`appx` or `winget`).
- Converter: `scripts/washmybelly/win-apps-convert.mjs --repos <folder>`; `--check` fails when `apps.json` is out of date.

Loaded by `src-tauri/omniget-core/src/core/washmybelly/apps/windows/catalog.rs` with `include_str!`; nothing is downloaded at run time.
