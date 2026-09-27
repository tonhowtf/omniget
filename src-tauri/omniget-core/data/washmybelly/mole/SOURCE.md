# Mole

- Repository: https://github.com/tw93/Mole
- Commit: 1b6ac63fe75b7ff2c05430742eacab5c78ef16cd (2026-09-27)
- Copied: 2026-09-27 by `scripts/washmybelly/sync_mole_protection.py`
- License: GPL-3.0 (see `LICENSE`). The name and logo are trademarks of the Mole project
  (`TRADEMARK.md`): they never appear in OmniGet's interface.
- `protection.txt`: every pattern of `SYSTEM_CRITICAL_BUNDLES`, `DATA_PROTECTED_BUNDLES` and
  `ENDPOINT_SECURITY_BUNDLE_PREFIXES`, plus the path patterns hard-coded in `should_protect_path`,
  `should_protect_data` and the default and safety whitelists of `lib/core/base.sh`.
- The macOS cleanup targets in `clean/rules_macos.rs` marked `source: "Mole"` were ported by hand
  from `lib/clean/{user,dev,app_caches,brew,system}.sh` of the same commit.
