# Kudu cleaner rules (Windows)

- Upstream: https://github.com/AdventDevInc/kudu (Kudu 3.4.0), license MIT (see `LICENSE` here).
- Commit: `4e0002259b58e8ee41925611831e8f4085295ddd` (2026-09-25).
- Read: 2026-09-27.
- Files converted: `rules/win32/{system,apps,gaming,gpu-cache,browsers}.json`.
- Converter: `scripts/washmybelly/kudu-convert.mjs --repo <kudu checkout>`; `--check` fails when `rules.json` is out of date.
- `rules.json` carries `commit`, `count` and a `skipped` list with the reason for every upstream target left out.

Loaded by `src-tauri/omniget-core/src/core/washmybelly/clean/kudu.rs` with `include_str!`; nothing is downloaded at run time.
