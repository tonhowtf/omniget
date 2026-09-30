# Settings frontend harness (v3)

This harness renders the real Svelte UI with fully synthetic IPC. It never opens the native app, uses an MCP grant, accesses a personal profile, sends provider requests, writes credentials, or installs extensions. It is **not evidence of native persistence or native accessibility**.

A Vite server must already be answering on `http://localhost:1431`; the harness never starts, discovers or stops one. Its request gate allows only GET/HEAD document/assets at that exact origin, blocks `/mcp`, `/v1`, `/api` and every external request, and closes every WebSocket except Vite HMR. Only `node_modules` and binary assets are cached across cases; `/src` modules always come fresh, because other sessions may be editing them while the run is in progress.

## Synthetic IPC (`mock-ipc.mjs`)

`installSyntheticIPC` answers every Tauri call in the page. Unknown commands reject with `HARNESS_UNHANDLED_COMMAND` and are recorded (`fixture-incomplete`), never answered with a fake success.

- **v3 contract** (`06-api-contract.md`): `settings_get_v3`, `settings_patch_v3`, `settings_reset_v3`, `settings_retry_effect_v3`, `settings_import_appearance_v3`, all taking `request` like the store sends it. Envelope in camelCase, revision as a decimal string (starts at `"7"`, `BigInt` inside the mock), `expectedRevision` CAS with `conflict` errors, `ApiError` objects rejected as plain data (like Tauri serializes them). The DTO is redacted: `proxy.has_password`, `advanced.has_twitter_manual_cookie`, `bridge.has_token`, no secret values. The private document carries fictitious sentinels (`SYNTHETIC-SECRET-*`); every case checks that they never reach the DOM or an input value. Appearance is already migrated: `mode`, `preset: "omniget-desktop"`, `legacy_theme: null`, `migration_version: 1` (the legacy `omniget.workspace.appearance.v1` key is present on purpose, so an import call would show up as a mutation).
- **`settingsState`**: `ready`, `loading` (get never resolves), `get-failed` (`FailedSnapshot` `read_failed`; mutators reject `unavailable/load_failed`), `patch-rejected` (`persistence/write_failed`, `committed: false`), `pending-effect` (one `network.proxy` effect `failed`, retryable; the first retry clears it), `recovered` (`loadStatus.kind = recovered`).
- **Legacy wrappers** `get_settings`/`update_settings`/`reset_settings` use the same document and return the redacted DTO.
- **Family states** (`family` + `state`): `loading` holds and `error` rejects the family's read commands; `empty`/`disabled`/`expanded` change fixtures. `delays` makes any command answer late.
- **Mutation counter**: every call is logged by name with `mutating: true|false`. Mutating = the explicit list `MUTATING` in the mock (settings v3 and legacy writes, onboarding/legal marks, AI config/test/summarize/history clear, rules, channels, profile, cookies, usage tray set/toggle/refresh, league, bridge rotation/pairing, extension export, dependency install, downloads, pet/world, all `tool_mcp_*` writes and grants, music import/playlist/like, `assist_learning_*` writes) plus any other command whose name has a write verb (`set|save|add|remove|delete|clear|create|update|reset|import|retry|install|rotate|revoke|grant|toggle|refresh|start|stop|send|download|export|…`) unless it reads (`get_*`, `list_*`, `*_status`, `*_view`, …). Session plumbing that runs on every load (`register_frontend`, `native_agent_register`, `sync_*_strings`, event listen/unlisten, …) is not counted.
- `installDestinationFixtures` adds read-only answers for the contextual destinations (roster with the bot `fixture-bot`, MCP status/clients/grants, music status/library, Home templates, limits strip, memory/reading overviews). Their writes stay unknown.

## Modes (`visual-audit.mjs`)

All modes run with concurrency 1 and a 30 s budget per case. A case that fails is retried once; both attempts are kept in the report (`attempt: 1|2`). Output goes to `docs/settings-rebrand/execution/evidence/<mode>/` (override with `--out=`; the audit's `docs/settings-rebrand/evidence/` is never written).

```sh
A=scripts/settings-rebrand/visual-audit.mjs
node $A --mode=navigation --languages=en,pt,fa --viewports=1100x720
node $A --mode=navigation --languages=en,pt,fa --viewports=800x560,1440x900 --variants=cold-late,client-late
node $A --mode=focus --languages=en,pt,fa
node $A --mode=baseline                      # 6 categories × 12 locales × 2 themes × 3 viewports = 432
node $A --mode=sections                      # 43 section ids × en,pt × dark,light at 1100×720 = 172
node $A --mode=aliases                       # navigation-map: 29 tab/section + 19 hashes + 9 acceptance + 146 settings = 203
node $A --mode=states                        # 48 category×state recipes × en,pt × dark,light = 192
node $A --mode=conditional                   # 8 prerequisite groups × expanded|click × en,pt × dark,light = 64
node scripts/settings-rebrand/summarize.mjs
node scripts/settings-rebrand/verify-evidence.mjs
```

Filters: `--languages=`, `--themes=`, `--viewports=`, `--categories=`, `--states=`, `--cases=` and `--variants=` (navigation), `--limit=`, `--resume=<coverage.json>` (skips cases with a recorded outcome), `--late-ms=` (default 1500).

- **baseline / sections**: open `/settings?tab=<category>[&section=<id>]`, wait for the shell and the end of the skeleton, scroll the section into view, measure and capture the viewport.
- **aliases**: every `tabSectionAliases`, `legacyHashAliases`, `acceptanceCases` and `settingAliases` entry of `navigation-map.json`. Checks the canonical URL (tab/section/setting, hash kept), the focus on the setting or section, and zero mutating calls.
- **states**: `loading`, `error` (settings profile unreadable: `FailedSnapshot`), `pendingApply` (banner, then one explicit retry: counts `settings_retry_effect_v3` and whether the banner goes away), `recovered`, `saving` (patch held), `saved`, `saveFailed` (persistence error), `dirty` (draft forms: typography, output, AI connection, RPC, proxy), `disabled`, `empty`, `partialError` (one panel's read fails). Combinations that do not exist are not invented (see `STATE_RECIPES`); `not-applicable` is recorded when the expected control is absent.
- **conditional**: prerequisite toggles and advanced disclosures, once with the prerequisites on in the fixture (`expanded`) and once by a real click on the switch (`click`), counting visible controls before/after.
- **focus**: Tab walk (every focused control must match `:focus-visible` and have an outline or box-shadow, and stay in the viewport), Shift+Tab reverses, disclosure toggled by Enter (`aria-expanded` and `hidden` agree), reset dialog: focus moves in, Escape closes without calling reset and returns focus to the trigger; Ctrl+F focuses the search, a result opens its target and "Back to results" restores the query.
- **navigation**: contextual destinations (`/llm/roster?bot=&section=skills|learning`, `/llm/mcp?mode=server&section=permissions|media`, `/music?view=settings[&panel=eq]`, `/?panel=download-advanced`, `/llm/summaries?section=form|history[&setting=]`, `/settings?tab=general&section=limits-strip|usage-tray`, the summaries alias, and negative cases with an unknown bot, mode or section). Variants: `cold` (the page is loaded at the deep link), `client` (a link clicked inside an already loaded Settings page), and `-late` versions where the reads the destination depends on answer 1.5 s late. The case waits for focus on the expected element, then checks that it is still there 1.5 s later (a router focus reset or re-render must not take it away), and counts mutating calls caused by the navigation (expected: zero).

## Measurements

Per case: document and container horizontal overflow (`scrollWidth > clientWidth` on shell, panel, groups, rows and the save bar), controls whose box leaves the content area (cut), controls without an accessible name (aria-label, aria-labelledby, `<label>`, text, title; a heuristic, not an accessibility-tree oracle), alerts/statuses/save bars, `lang`/`dir`, `data-theme`/`data-preset`, focus trail, and secret sentinels. For non-English locales, visible strings that are exactly an English catalog value missing from the locale's catalog are listed as `englishLeaks` (brand names that are equal in both catalogs are ignored).

Statuses: `captured-needs-review` (screenshot taken, nothing proven visually), `not-applicable`, `focus-missed`, `focus-lost`, `url-mismatch`, `mutation-on-navigation`, `assertion-failed`, `section-missing` (product outcomes), `fixture-incomplete` (an unknown command was called), `failed` (the harness could not finish the case after the retry).

`summarize.mjs` merges every `coverage-*.json` of the evidence folder (latest attempt per case id) into `summary.json` and `summary.md`. `verify-evidence.mjs` writes `coverage-merged.json/.csv` and `integrity.json`: PNG signature, SHA-256 and viewport-sized dimensions of every authoritative screenshot, planned vs observed cases per mode, and `lang`/`dir` per case. It exits nonzero when an image fails, a locale mismatches, or a mode that ran is incomplete. This is file integrity, not visual approval: a screenshot counts as reviewed only when it was opened and described in the handoff.
