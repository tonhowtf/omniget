# Contributing to OmniGet

Thanks for helping. This page is the short version of how the project works: how to get it running, what the checks are, and the few rules that keep the codebase tidy. Questions go to [Discord](https://discord.gg/jgdxyPy7Vn); bugs and ideas go to [Issues](https://github.com/tonhowtf/omniget/issues).

## Contents

- [Where to start](#where-to-start)
- [Setup](#setup)
- [Dev loop](#dev-loop)
- [Checks](#checks)
- [Tests](#tests)
- [Project rules](#project-rules)
- [Who decides](#who-decides)
- [Commits and pull requests](#commits-and-pull-requests)
- [AI-assisted contributions](#ai-assisted-contributions)
- [Environment variables](#environment-variables)

## Where to start

- **Read [ARCHITECTURE.md](ARCHITECTURE.md)** first. It maps the crates, shows where a Tauri command and an MCP tool live, and follows one download from the pasted link to the file.
- **Pick something small.** Issues labelled [`good first issue`](https://github.com/tonhowtf/omniget/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) are scoped for a first pull request, and [`help wanted`](https://github.com/tonhowtf/omniget/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22) ones are open to anyone. Comment on the issue before you start, so two people do not do the same work.
- **Translate.** The interface is translated on [Weblate](https://hosted.weblate.org/engage/omniget/), in the browser, with no setup.
- **Bigger idea?** Open a [discussion](https://github.com/tonhowtf/omniget/discussions) or an issue before writing code. The public board at [getomniget.com/contact](https://getomniget.com/contact) shows what is already being worked on.

## Setup

| Tool | Version | Where it comes from |
|---|---|---|
| [Rust](https://rustup.rs/) | pinned | `rust-toolchain.toml`; rustup installs it on the first `cargo` call |
| [Node.js](https://nodejs.org/) | 20, 22 or 24+ | `.nvmrc` has the recommended version (`nvm use`, `fnm use`) |
| [pnpm](https://pnpm.io/) | pinned | `packageManager` in `package.json`; `corepack enable` picks it up |

On Linux, install the system libraries listed under [Build from source](README.md#build-from-source) first.

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

Use pnpm. `package-lock.json` exists only for the Flatpak build and the version bump; do not update it by hand.

## Dev loop

- **`pnpm tauri dev`** runs the app with hot reload for the frontend and a rebuild on Rust changes. A debug build uses its own profile, `omniget-dev` in your platform data folder (`~/Library/Application Support` on macOS, `~/.local/share` on Linux, `%APPDATA%` on Windows), so it never reads or writes the settings, keys, history and databases of the OmniGet you have installed, and it can run next to it. It also keeps secrets in the encrypted file store inside that profile instead of the OS keyring, keeps its WebView data (logins, local storage) in `webview/` inside it on Windows and Linux, and takes its bridge port from 47760 to 47769, so it never takes the installed app's port (47720 to 47729) or the extension pairing with it. Two things are still shared: the unpacked extension that **Browser extension** settings export, and the autostart entry if you turn autostart on from a dev build. Set `OMNIGET_DEV_REAL_PROFILE=1` to use your real profile again, or `OMNIGET_DATA_DIR=/some/folder` to pick another one.
- **`pnpm dev:app`** is the supervised loop for scripts and automation: one Vite process, one Cargo watcher, an isolated profile, and JSON commands to wait for a revision and take a screenshot (`pnpm dev:wait`, `pnpm dev:capture`, `pnpm dev:stop`). See [scripts/dev/README.md](scripts/dev/README.md). Do not run `pnpm tauri dev` against the same session.
- **`pnpm dev`** runs only the Vite frontend, without the Rust side.

## Checks

Run these before you push. CI runs the same checks on every pull request and has to pass before a merge, and a full run takes a long time, so running them first saves you a round trip.

| Command | What it does |
|---|---|
| `pnpm verify` | Every check, in one command, with one table of results at the end |
| `pnpm verify --changed` | Only the areas your branch touched since it left `main` (commits, working tree and new files) |
| `pnpm verify --fix` | Runs `cargo fmt --all` first |
| `pnpm hooks:install` | Opt-in: a pre-push hook (`scripts/git-hooks/`) that runs `pnpm verify --changed`. `pnpm hooks:uninstall` removes it; `git push --no-verify` skips it once |

`pnpm verify` builds Rust in `~/.cache/omniget/target`, shared by every checkout and worktree, so a second clone does not pay the cold build again. `CARGO_TARGET_DIR` still wins, and `OMNIGET_SHARED_TARGET=0` goes back to `src-tauri/target`. They use [sccache](https://github.com/mozilla/sccache) when it is installed.

The individual steps, if you want to run one by hand:

- Frontend: `pnpm check` (svelte-check), `pnpm test` (vitest), `pnpm check:i18n`
- Rust, inside `src-tauri/`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`
- Clippy baseline, from the root: `pnpm check:clippy-baseline`
- Tauri commands and events: `pnpm check:ipc`
- Browser extension: `pnpm test:extension`. Claude Code plugin: `pnpm test:plugin`

## Tests

- **Tests guard what breaks silently.** The suite keeps parsers and extractors of platforms, HLS/DASH and file formats, accounts and credentials, and migrations of data on disk. Add a test there when you fix a bug; glue, UI and logic the type checker already covers do not get tests.
- **No real clocks.** Never sleep to wait for a timer. In async Rust use tokio's paused time (`#[tokio::test(start_paused = true)]` or `tokio::time::pause()` with `advance`); elsewhere pass the duration in as a parameter so the test can use a tiny one.
- **Name tests after behavior.** No tests tied to plan items, ticket numbers or steps of a roadmap: a test exists because the behavior matters, and its name says which behavior.
- **No network and no real machine.** A test that needs the network, a real agent CLI or reads the real system does not go in the suite.
- **Never touch the real profile.** Point `OMNIGET_DATA_DIR` (and the secret-store variables) at a temporary folder.

## Project rules

- **Clippy baseline.** `scripts/clippy-baseline.mjs --check` fails on any new warning. Fix the warning; never run `pnpm update:clippy-baseline` to hide one of yours.
- **Interface text.** A key is born in `src/lib/i18n/en.json`. The other languages come from [Weblate](https://hosted.weblate.org/engage/omniget/) or from translation PRs; do not add machine translations to them in a feature PR. Run `pnpm generate:i18n-keys` and commit the generated `src/lib/i18n/keys.ts`.
- **Tauri commands and events.** A new command is defined, registered in `generate_handler!` in `src-tauri/src/handlers.rs` and called from the frontend in the same PR; a new frontend permission goes in `src-tauri/capabilities/`. The name Rust emits (`app.emit("x")`) must be exactly the name the frontend listens to (`listen("x")`). `pnpm check:ipc` catches drift.
- **Browser extension.** Edit `browser-extension/chrome/`, then run `pnpm extension:sync` to copy the shared files to `firefox/`.
- **Removing a feature removes everything only it used:** Rust commands and their registration, `invoke` calls, components, stores, i18n keys, assets in `static/`, capabilities and system permissions (`Info.plist`, `Entitlements.plist`, `.deb` depends), dependencies in `Cargo.toml` and `package.json`, and its README section and images. Grep for the feature's name afterwards.
- **READMEs.** `README.md` is the source. `README.pt-BR.md`, `README.zh-CN.md` and `README.ru.md` keep the same sections in the same order; change all four in the same PR, leaving English text where you cannot translate.
- **Similar code.** Follow the style of the file around you, including how much it comments and in which language.

## Who decides

- OmniGet is maintained by Antonio Leandro ([tonhowtf](https://github.com/tonhowtf)), who reviews and merges every pull request and decides what goes into a release. [GOVERNANCE.md](GOVERNANCE.md) has the details, including what depends on one person.
- Changes come in through pull requests, and CI has to pass before a merge.
- Ideas and questions go to [Discussions](https://github.com/tonhowtf/omniget/discussions), bugs to [Issues](https://github.com/tonhowtf/omniget/issues), and security problems to the address in [SECURITY.md](SECURITY.md).
- Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Commits and pull requests

- Messages in English, [Conventional Commits](https://www.conventionalcommits.org/): `feat(scope): ...`, `fix`, `refactor`, `perf`, `docs`, `chore`, `style`, `test`. Add `!` for a breaking change (`refactor!: remove plugins`).
- Sign off your commits (`git commit -s`). The `Signed-off-by` line says you wrote the change or have the right to send it under GPL-3.0, in the words of the [Developer Certificate of Origin](https://developercertificate.org/). There is no CLA to sign.
- One pull request per subject. Say what changes for the person using the app and how you tested it; the PR template asks for both.
- Only the product goes into git: code, build configs, lockfiles, workflows and assets the app or the READMEs use. Keep personal notes, plans and scratch files out of commits; list them in your own `.git/info/exclude` instead of `.gitignore`.

## AI-assisted contributions

You may use an AI tool to write code, tests or text for OmniGet. Much of the app itself is built that way. The rules are about responsibility, not about the tool:

- **You are the author.** You have read every line you send, you can explain it, and you answer the review yourself.
- **You ran it.** `pnpm verify` passes on your machine and you tried the change in the app. A pull request that was never run gets closed.
- **Say so.** Tick the box in the pull request template if an AI tool wrote a meaningful part, and say what you checked by hand.
- **Keep it small.** One subject per pull request applies twice here. A large generated diff nobody asked for is not reviewed.
- **Reports too.** An issue or a security report has to describe something you saw happen. Reports that an AI tool produced from reading the code, without a reproduction, are closed.

## Environment variables

None of these are needed for normal development. The ones a contributor or a power user might set:

| Variable | What it does |
|---|---|
| `OMNIGET_DATA_DIR` | Runs the app (and `omniget-cli`) against this folder instead of the default profile. A hand-set folder is a second profile: own settings, own bridge port, no single-instance lock, so it runs next to the installed app. |
| `OMNIGET_DEV_REAL_PROFILE` | `1`: a debug build (`pnpm tauri dev`) uses the real profile instead of `omniget-dev`. |
| `OMNIGET_KEYRING` | `1` or `0`: forces the OS keyring on or off for stored secrets. Default: off on macOS unless the binary has a stable signature, on in Windows release builds, on for Linux (Secret Service, falling back to the encrypted file). Also read under its old name `OMNIGET_OMNIDISC_KEYRING`. |
| `OMNIGET_YTDLP_CHANNEL` | `nightly`: download yt-dlp from the nightly builds instead of the stable release. |
| `OMNIGET_POT_PROVIDER_URL` | Base URL of a running YouTube PO token provider (bgutil); yt-dlp is pointed at it. |
| `OMNIGET_PYTHON` | Python 3.10+ to try first for running yt-dlp as a zipapp (much faster to start than the single-file build on macOS). |
| `OMNIGET_RELAY` | `host:port` of the relay used to send files between devices. Default `relay.tonho.wtf:9009`. |
| `OMNIGET_EMBED_MODEL_DIR` | Folder that already holds the local embedding model files, so they are not downloaded. |
| `OMNIGET_SKILL_SCANNER` | Path to the `skillspector` binary used to scan installed skills, when it is not on `PATH`. |
| `OMNIGET_SKILL_SCAN` | `0`, `off` or `false`: skip the skill scan. |
| `OMNIGET_MCP_URL`, `OMNIGET_MCP_TOKEN` | Endpoint and token the `omniget-mcp` stdio adapter uses to reach the desktop app. The URL must be `http://127.0.0.1`, `localhost` or `[::1]` with path `/mcp`. |
| `OMNIGET_DEV_ENDPOINT` | Points `omniget-mcp` at a `pnpm dev:app` session's `endpoint.json` instead of the installed app. |
| `OMNIGET_NO_SPLASH` | Any value: `omniget claude` does not print the splash (same as `--no-splash`). |
| `OMNIGET_I18N_STRICT` | `1`: `vite dev` fails on locale key problems, as `vite build` does. `0`: `vite build` does not. |
| `OMNIGET_SHARED_TARGET` | `0`: `pnpm verify` builds in the checkout's own `src-tauri/target` instead of the shared `~/.cache/omniget/target`. `CARGO_TARGET_DIR` still wins. |
| `OMNIGET_TEST_DRIVER` | `1`, debug builds only: turns on the local bridge's `POST /v1/debug/eval` test driver. `pnpm dev:app` sets it. |

The Claude Code plugin has its own variables (`OMNIGET_DIR`, `OMNIGET_TOOL_*`, `OMNIGET_WHISPER_MODEL`, `OMNIGET_COOKIES_FROM_BROWSER`, `OMNIGET_KEYS_FILE`); see [claude-plugin/omniget/README.md](claude-plugin/omniget/README.md#environment-variables).

### Test-only and internal

- Test isolation: `OMNIGET_SECRETS_DIR` (every secret store under one folder), `OMNIGET_PROFILE_SECRET_DIR`, `OMNIGET_OMNIDISC_SESSION_DIR`, `OMNIGET_AI_KEYS_SECRET_DIR`, `OMNIGET_MCP_DOWNLOADS_SECRET_DIR`, `OMNIGET_LIMITS_DIR`, `OMNIGET_PET_DIR`, `OMNIGET_WIZARD_ROOT` (`OMNIGET_WIZARD_KEEP`), `OMNIGET_CLITOOLS_EXTRA_PATH`, `OMNIGET_CLITOOLS_NPM_PREFIX`.
- Generators and the smoke test: `pnpm gen:mcp-catalog` (regenerates `omniget-cli/mcp-catalog.json` after an MCP tool changes), `OMNIGET_SMOKE_EXIT_MS` (`pnpm smoke`).
- Set by OmniGet for its own child processes; do not set them: `OMNIGET_PORTABLE` (portable mode, turned on by a `portable.txt` next to the executable), `OMNIGET_DEV_PROFILE`, `OMNIGET_DEV_SESSION`, `OMNIGET_DEV_URL`, `OMNIGET_BACKEND_BUILD_ID`, `OMNIGET_WORKER_BUILD_ID`, `OMNIGET_BOOT_ID`, `OMNIGET_WORKER_MODE` and the other `OMNIGET_WORKER_*`, `OMNIGET_INFO_JSON_DIR`, `OMNIGET_LAUNCH_ID`, `OMNIGET_ARENA`, `OMNIGET_PROJECT_ROOT`, `OMNIGET_WORKTREE_PATH`, `OMNIGET_WMB_*`.
