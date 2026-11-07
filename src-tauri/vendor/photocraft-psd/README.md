# photocraft-psd (vendored)

PSD and PSB reader used by the Converter to open Photoshop files.

- Origin: https://github.com/storytold/photocraft, crate `crates/psd`
- Commit: `c101cbfb7ea556b248d4c7f4884dd5816a0b8a74` (2026-11-06)
- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`)

What was cut, because the Converter only reads PSD and PSB:

- The modules for other Adobe formats: `abr`, `aco`, `act`, `ase`, `asl`, `atn`, `csh`, `grd`, `shc`, `hdr`, `patterns`, `slices`, `metadata`, `path` and the layered TIFF transcoder `tiff`.
- `codepage` (legacy code page guessing used only by those formats). The one call left in `file.rs` decodes Mac Roman directly.
- The helpers only those modules used (`utf16_units`, `Reader::new_little`); `append_with` and `write_unpadded` are now test-only.
- The `corpus` feature, the integration tests in `tests/`, the fuzz targets and the examples.
- Workspace inheritance: version, edition and dependencies are written out in `Cargo.toml`.

Nothing else was changed. The crate is not a member of the omniget workspace (it is wired in through `[patch.crates-io]` in `src-tauri/Cargo.toml`), so it keeps its own `rustfmt.toml` and lints.
