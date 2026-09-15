import { defineConfig } from "vitest/config";
import path from "path";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
    globals: false,
  },
  // `npm test` has to work on a fresh checkout. The root tsconfig.json extends
  // the generated .svelte-kit/tsconfig.json, which only exists after
  // `svelte-kit sync` (what `pnpm check` runs before `pnpm test` in CI). When it
  // is missing, vite's esbuild transform blows up while resolving `extends` and
  // no test file is ever collected. Handing esbuild the same options that
  // generated file contributes (target + verbatimModuleSyntax, see the
  // compilerOptions below) keeps the transform identical and drops the
  // dependency on the generated file.
  esbuild: {
    tsconfigRaw: JSON.stringify({
      compilerOptions: {
        target: "esnext",
        verbatimModuleSyntax: true,
      },
    }),
  },
  resolve: {
    alias: {
      $lib: path.resolve(__dirname, "src/lib"),
      $components: path.resolve(__dirname, "src/components"),
    },
  },
  define: {
    __COMMIT_HASH__: JSON.stringify("test-commit"),
    __GIT_BRANCH__: JSON.stringify("test-branch"),
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
    __BUILD_DATE__: JSON.stringify("2026-04-13"),
  },
});
