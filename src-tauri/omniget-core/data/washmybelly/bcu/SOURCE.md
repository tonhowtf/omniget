# Bulk Crap Uninstaller (design ported, no data)

- Upstream: https://github.com/Klocman/Bulk-Crap-Uninstaller, license Apache-2.0 (see `LICENSE` and `NOTICE` here).
- Commit read: `30da609384c98ba6e35c6530129541ea4ff3970a` (2026-09-08).
- Read: 2026-09-27.
- Nothing from BCU is embedded as a file. Its design was ported to Rust, with an attribution header, in
  `src-tauri/omniget-core/src/core/washmybelly/apps/windows/`:
  - inventory steps and duplicate merge by score (`Factory/ApplicationUninstallerFactory.cs`, `Factory/ApplicationEntryTools.cs`);
  - Uninstall registry values, update/hotfix and GUID rules (`Factory/RegistryFactory.cs`);
  - installer kind cascade (`Factory/InfoAdders/UninstallerTypeAdder.cs`);
  - quiet uninstall strings for MSI and Inno Setup, the NSIS `_?=` trick (`Uninstaller/UninstallManager.cs`, `Factory/InfoAdders/*QuietUninstallStringGenerator.cs`);
  - one Windows Installer job and one windowed uninstaller at a time, stall detection by CPU and disk, exit code meanings (`Uninstaller/BulkUninstallTask.cs`, `Uninstaller/BulkUninstallEntry.cs`);
  - leftover confidence weights, levels and folder blacklist (`Junk/Confidence/*.cs`, `UninstallToolsGlobalConfig.cs`, `Junk/Finders/*`).
- Changes: rewritten in Rust; winget matching and removal added (BCU has none); NSIS runs `/S` instead of UI automation; leftover files go to the Trash and registry keys are exported to `.reg` before removal; everything runs through washmybelly plans and receipts.
