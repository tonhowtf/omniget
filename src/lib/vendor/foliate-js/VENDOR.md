# foliate-js (vendored)

- Source: https://github.com/readest/foliate-js (MIT), the Readest fork of johnfactotum/foliate-js.
- Commit: `1f77f389e901d86c7d6c2c85535a4753bf51b51d` (2026-09-29).
- Only the library modules are copied; `reader.js`, `ui/`, `tests/` and `vendor/pdfjs/` are left out.

Local patches:
1. `fixed-layout.js`: dropped `import 'construct-style-sheets-polyfill'`.
2. `pdf.js`: `import('@pdfjs/pdf.min.mjs')` -> `import('pdfjs-dist/legacy/build/pdf.min.mjs')`.
   The pdf.js runtime files it loads from `/vendor/pdfjs/` are copied by `scripts/setup-pdfjs.mjs`.
3. Every `.js` file starts with `// @ts-nocheck` (the repo type-checks JS with `checkJs`).
