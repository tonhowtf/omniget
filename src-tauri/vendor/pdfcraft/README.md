# pdfcraft (phase 1)

Source: https://github.com/storytold/pdfcraft at commit `f42fda734243b66f54db87923d7c981d5221419e` (version 0.18.1).
License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.

Used by `omniget-core/src/core/pdf` to read, reorder, merge, split, rotate and save PDFs.

| Folder | Upstream path | Why |
|---|---|---|
| `cos` | `crates/cos` | Tolerant PDF reader and writer |
| `crypt` | `crates/crypt` | Opens PDFs protected only by an owner password |
| `filters` | `crates/filters` | Stream filters used by `cos` |
| `content` | `crates/content` | Content streams, needed by `organize` |
| `organize` | `crates/organize` | Combine, extract, rotate, insert blank pages |
| `hayro-syntax` | `vendor/hayro-syntax` | hayro-syntax 0.7.2 with pdfcraft's patches (`PdfCraft patch:` in the source) |
| `hayro-jbig2` | `vendor/hayro-jbig2` | hayro-jbig2 0.3.0 with pdfcraft's patches; the crates.io release lacks the API `filters` calls |

The hayro crates are by Laurenz Stampfl (https://github.com/LaurenzV/hayro), Apache-2.0 OR MIT; their license files are in their folders.

What changed from upstream:

- Every `src/` folder is byte-for-byte the upstream commit. The `tests.rs` files stay in place but are never compiled: these crates are not workspace members (they are wired in through `[patch.crates-io]` in `src-tauri/Cargo.toml`, because Cargo makes every plain path dependency inside `src-tauri/` a member, `exclude` or not) and their test-only crates (`pdfcraft-oracle`, `pdfcraft-testkit`, `proptest`) were not copied.
- Left out: `tests/`, `examples/`, `fuzz/`, benches, and every other pdfcraft crate (render, optimize, create, xfa, engine, ui and the rest).
- Each `Cargo.toml` was rewritten without workspace inheritance, with path dependencies between these folders, no dev-dependencies and `clippy::all` allowed. `hayro-jbig2` no longer enables the `image` feature by default, and `content` uses `hayro-syntax` without its image decoders, which neither needs.

Updating: copy the same folders from a newer commit, keep the manifests, run `cargo test -p omniget-core core::pdf`, and change the commit above.
