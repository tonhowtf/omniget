# Contributing to OmniGet

Thanks for taking the time to contribute.

## Running the dev build

**Prerequisites:** [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) 18+, [pnpm](https://pnpm.io/) 10+.

On Linux, install the Tauri system dependencies first:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

Then:

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

## Before opening a pull request

Run these locally — CI runs the same checks:

```bash
cd src-tauri
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace

cd ..
pnpm check
```

## Adding a translation

Translations live in two places:

**Main app** (`src/lib/i18n/`):

1. Copy `en.json` to `<locale>.json` (e.g. `es.json`) and translate the values.
2. Register the locale in `src/lib/i18n/index.ts` by adding an entry to the `loaders` array:

   ```ts
   {
     locale: "es",
     key: "",
     loader: async () => (await import("./es.json")).default,
   },
   ```

3. Add a `lang_<locale>` entry (e.g. `"lang_es": "Español"`) under `settings.appearance` in **every** `src/lib/i18n/*.json` file.
4. Add a matching `<option>` to the language selector in `src/routes/settings/+page.svelte`:

   ```svelte
   <option value="es">{$t('settings.appearance.lang_es')}</option>
   ```

5. Regenerate the translation key types:

   ```bash
   pnpm generate:i18n-keys
   ```

**Browser extension** (`browser-extension/chrome/_locales/` and `browser-extension/firefox/_locales/`):

1. Create an `<locale>/` folder in both (e.g. `es/`).
2. Copy `en/messages.json` into it and translate the `message` fields. Leave the keys and `description` fields unchanged.

Run `pnpm check` before opening the PR.

## Commit style

Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`. Keep the subject under 72 characters.

## Security issues

Do not file public issues for security problems. See [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your changes are licensed under [GPL-3.0](LICENSE).
