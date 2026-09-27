# winutil Store ids (Windows)

- Upstream: https://github.com/ChrisTitusTech/winutil, license MIT (see `LICENSE` here, Copyright (c) 2022 CT Tech Group LLC).
- Commit: `ad81e0a7aff424932afff77504e7440e07e19a15` (2026-09-26).
- Read: 2026-09-27.
- File converted: `config/appx.json`, only `PackageId` -> `StoreId` (31 packages), used to reinstall a removed Appx with `winget install --id <StoreId> --source msstore`.
- The winget exit code meanings in `apps/windows/parse.rs` follow `functions/private/Install-WinUtilProgramWinget.ps1` (facts, rewritten).
- Converter: `scripts/washmybelly/win-apps-convert.mjs --repos <folder>`; `--check` fails when `appx.json` is out of date.

Loaded by `src-tauri/omniget-core/src/core/washmybelly/apps/windows/catalog.rs` with `include_str!`; nothing is downloaded at run time.
