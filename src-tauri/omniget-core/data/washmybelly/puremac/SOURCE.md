# PureMac

- Repository: https://github.com/momenbasel/PureMac
- Commit: 64d4d7fe645048440959c250a38f6b7428a9d633 (2026-09-26)
- License: MIT, Copyright (c) 2026 PureMac Contributors (see `LICENSE`).
- Used: the macOS cleanup targets of the CLI catalog (`cli/Sources/puremac/Core/Catalog.swift`) and of
  `PureMac/Services/ScanEngine.swift` (Trash of each volume, Docker, Xcode, AI apps), ported by hand into
  `clean/rules_macos.rs` as rules with `source: "PureMac"`, `license: "MIT"`.
- Not used: `PureMac/Logic/Scanning/Conditions.swift` and anything derived from Pearcleaner
  (Apache-2.0 + Commons Clause, incompatible with GPL-3.0).
