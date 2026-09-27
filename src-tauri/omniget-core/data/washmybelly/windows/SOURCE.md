# Windows cleanup rules written for washmybelly

The hand-written Windows rules live in code, in
`src-tauri/omniget-core/src/core/washmybelly/clean/rules_windows.rs`, and are ours (GPL-3.0, like OmniGet).

Where the paths and the safety ideas came from (facts and designs, no code copied):

- Kudu (MIT, https://github.com/AdventDevInc/kudu, commit `4e0002259b58e8ee41925611831e8f4085295ddd`): `minAgeDays`, the Restart Manager lookup of who holds a file (`src/main/services/cleaner-blockers.ts`), the Delivery Optimization cmdlet.
- BitBroom (MIT, https://github.com/pwnapplehat/BitBroom): protected trees, never follow junctions, revalidate before delete, junction canary test.
- winutil (MIT) and Sophia Script (MIT): what they clean in `%TEMP%` and `C:\Windows\Temp`; their `DISM /ResetBase` and `cleanmgr` steps are left out on purpose.
- Read on 2026-09-27.
