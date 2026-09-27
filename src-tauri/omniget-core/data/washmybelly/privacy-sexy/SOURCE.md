# privacy.sexy collections

- Repository: https://github.com/undergroundwires/privacy.sexy
- Version: 0.13.8, commit `7b956f3d8c8b80bbb2b5d0f77912431bf98fdafb` (2025-04-18)
- Files: `src/application/collections/{windows,macos,linux}.yaml`, copied byte for byte
- Copied: 2026-09-27 by `scripts/washmybelly/privacy_sync.sh`
- License: GNU AGPL-3.0 (see `LICENSE`). This folder stays AGPL-3.0; OmniGet (GPL-3.0) combines
  with it under section 13 of both licenses.
- Source of this part for people using OmniGet over a network (MCP over HTTP): the link above and
  https://github.com/tonhowtf/omniget (`src-tauri/omniget-core/data/washmybelly/privacy-sexy/`).

The YAML files are never edited here. OmniGet compiles them in Rust
(`src-tauri/omniget-core/src/core/washmybelly/privacy/compiler.rs`, written from the format
documentation, not translated), runs each item on its own through typed operations, and reads
each item's state from the system. Golden tests compare the Rust compiler with the original
TypeScript compiler (`scripts/washmybelly/privacy_golden.sh`).
