# Browser preview

`pnpm dev:web` opens the whole UI in the browser at `http://localhost:1420`, with Vite
HMR and no Rust build. Use it for layout, copy, flows and empty states. A pill at the
bottom says it is the preview; click it to list the commands that had no data.

There is no backend. Every `invoke` is answered in the page, in this order:
`src/lib/dev/browser-preview/fixtures.ts` (written by hand; settings persist in
localStorage), then an empty value inferred from the `invoke<T>` type at the call site,
then a rejection that the UI handles as a real error. To see a screen with data, add its
command to `fixtures.ts`. Nothing is downloaded or written to disk, so test real behavior
with `pnpm dev:app` below. The preview only installs in Vite dev outside Tauri. It never
reaches a production build or the native window.

# Native development loop

From the repository root, run `pnpm dev:app`. Keep this terminal open. The session
owns one Vite process and one Cargo lifecycle watcher. Do not also run `tauri dev`
against that session. Default Vite port: 1430; override with `--port 1431`.
Build deadline defaults to 900000 ms; override with `--build-timeout`.
Use `--session /absolute/path` consistently to select a different session.
The app is real Tauri, with an isolated profile under `docs/dev-experience/session`.
The supervisor never opens the personal profile or resumes downloads/jobs.

After saving an edit:

```sh
pnpm dev:revision
pnpm dev:wait --revision <requestedRevision> --timeout 180000
pnpm dev:capture --revision <requestedRevision> --out /absolute/path/proof.png
pnpm dev:status
pnpm dev:doctor
pnpm dev:stop
```

Use the `requestedRevision` JSON string from `dev:revision`. Never substitute
semver, UI stateRevision or the last successful build. Commands emit JSON; use
`node scripts/dev/session.mjs ...` directly to omit pnpm's banner. Wait/capture
have a deadline and support Ctrl-C. Build diagnostics and process logs survive
Rust errors/restarts in the session directory. A failed build marks the old app
stale. The wait fails, even when the old window remains usable.

`dev:status` reports lifecycle phase separately from verified readiness/revision.
`pnpm dev:prepare` requests a canonical warm preparation; identical artifacts keep
the app running and do not recopy the worker. Supervisor implementation changes
require stopping and starting the session; its digest is recorded in state.

Frontend changes use Vite HMR. The WebView acknowledges a source digest, separately
from its semantic UI epoch. Rust changes use a canonical host/dev Cargo invocation
for app, worker and stdio adapter, then launch private copies of the artifacts.
Files embedded by Rust through `include_str!`/`include_bytes!` are also backend
inputs, even when they are CSS/JSON/assets, and necessarily trigger Cargo.
The supervisor restores the last nonsensitive route; it does not replay actions,
form edits, jobs, downloads or external requests. Settings, permissions and data
remain in this isolated profile. To discard the profile, stop its session first.

To use the existing persistent stdio adapter, configure your MCP client with:

```json
{
  "command": "/absolute/repository/docs/dev-experience/session/omniget-mcp",
  "env": {
    "OMNIGET_DEV_ENDPOINT": "/absolute/repository/docs/dev-experience/session/endpoint.json"
  }
}
```

The adapter re-reads the endpoint and verifies session/boot identity on every
request. It stays alive across restarts. During compilation it reports unavailable;
retry observation after `dev:wait`. Never blindly retry a mutation whose outcome is
unknown. The production MCP gains no shell/build/eval operations. Provisioning
uses the existing debug-only test driver, only inside the isolated profile.
Do not publish session files: credentials are local secrets, mode 0600.

Readiness checks navigation, document, fonts, visible images, marked data surfaces
and canvas adapters. `data-agent-readiness` accepts `loading`, `ready`, `streaming`,
`error`. Canvas adapters must declare readiness. Add route-specific data markers
when adding async surfaces. Loading/streaming/error/blocked states remain inspectable;
only ready surfaces can produce accepted screenshots. Readiness does not replace
assertions about the intended scenario or a visual review of the native image.

## Prepared test runs

`pnpm bench:run --only bn1 --quick` prepares app+core libtests once and passes their
explicit executables to children. No paid benchmark runs unless explicitly selected.
For repeated execution:

```sh
pnpm bench:prepare --out /absolute/path/prepared-run
pnpm bench:run --prepared /absolute/path/prepared-run/prepared.json --only bn1 --quick
```

Preparation captures Cargo JSON executables, source digests (including untracked
sources), configuration, toolchain, artifact hashes and runtime loader paths.
Changed code, changed configuration, missing/changed executables and zero selected
tests fail closed. Prepared libtests use each package directory as cwd. This is not
a doctest or integration-test adapter. The full workspace gate remains mandatory.
Do not reuse iteration results as optimized performance comparisons. The unidentified
external comparison runner still needs a dedicated integration.

No cache is cleaned. Cargo incremental remains enabled. A relocated checkout may
rebuild despite a cloned cache. Avoid concurrent benchmark/build activity when
collecting performance numbers. `pnpm test:dev` checks infrastructure contracts;
normal `pnpm check`, `pnpm test` and Rust workspace gates remain in place.

Native execution and direct-libtest loader behavior have been validated on macOS
Apple Silicon. Other platforms require their own native evidence. See
`docs/dev-experience/execution-report.md` for measurements and remaining limits.
