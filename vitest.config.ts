import { defineConfig } from "vitest/config";
import path from "path";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
    globals: false,
  },
  resolve: {
    alias: {
      $lib: path.resolve(__dirname, "src/lib"),
    },
  },
  define: {
    __COMMIT_HASH__: JSON.stringify("test-commit"),
    __GIT_BRANCH__: JSON.stringify("test-branch"),
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
    __BUILD_DATE__: JSON.stringify("2026-04-13"),
  },
});
