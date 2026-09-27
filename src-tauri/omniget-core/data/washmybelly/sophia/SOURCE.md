# Sophia Script: Appx never removed (Windows)

- Upstream: https://github.com/farag2/Sophia-Script-for-Windows, license MIT (see `LICENSE` here, Copyright (c) 2023 farag2).
- Commit: `1b761b48e3f680b6669c37ad61d92518d583a44e` (2026-09-24).
- Read: 2026-09-27.
- Extracted: the `$ExcludedAppxPackages` list of every `src/*/Module/Sophia.psm1` (union of the editions), 33 package names.
- Converter: `scripts/washmybelly/win-apps-convert.mjs --repos <folder>`; `--check` fails when `excluded_appx.json` is out of date.

Loaded by `src-tauri/omniget-core/src/core/washmybelly/apps/windows/catalog.rs` with `include_str!`; a package on this list is shown but never offered for removal.
