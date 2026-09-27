# OmniGet's own cleanup data

- Repository: https://github.com/tonhowtf/omniget
- License: GPL-3.0 (see `LICENSE`, the same as the rest of OmniGet).
- `protection.txt`: paths no cleanup rule may remove, added where the studies found the upstream
  lists short (cloud providers, credentials in dot folders, models and environments, games, OmniGet's
  own data). Ideas, not text, from PureMac's provider list.
- The rules themselves live in `clean/rules_ours.rs`, `clean/rules_macos.rs` and `clean/rules_linux.rs`.
