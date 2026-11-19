import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { execSync, spawnSync } from "child_process";
import { readFileSync, existsSync } from "fs";
import path from "path";
import { fileURLToPath } from "url";

import { devRevisionPlugin, previewDefaultsPlugin } from './scripts/dev/vite-plugin.mjs';
import { setupPdfjs } from './scripts/setup-pdfjs.mjs';

const host = process.env.TAURI_DEV_HOST;
const configDir = path.dirname(fileURLToPath(import.meta.url));

function getGitInfo() {
  try {
    return {
      hash: execSync("git rev-parse HEAD").toString().trim(),
      branch: execSync("git rev-parse --abbrev-ref HEAD").toString().trim(),
    };
  } catch {
    return { hash: "unknown", branch: "unknown" };
  }
}

function getVersion() {
  try {
    return JSON.parse(readFileSync("package.json", "utf8")).version;
  } catch {
    return "0.0.0";
  }
}

function i18nCheckPlugin({ strict } = { strict: false }) {
  return {
    name: "omniget-i18n-check",
    buildStart() {
      const script = path.join(configDir, "scripts", "check-i18n.js");
      if (!existsSync(script)) {
        return;
      }
      const args = strict ? [script, "--strict"] : [script];
      const result = spawnSync(process.execPath, args, {
        cwd: configDir,
        stdio: "inherit",
      });
      if (result.status !== 0) {
        throw new Error(
          `check-i18n.js exited with code ${result.status}. ` +
            `Fix locale keys or set OMNIGET_I18N_STRICT=0 to bypass.`,
        );
      }
    },
  };
}

function pdfjsRuntimePlugin() {
  return {
    name: "omniget-pdfjs-runtime",
    buildStart() {
      setupPdfjs();
    },
  };
}

export default defineConfig(({ command }) => {
  const gitInfo = getGitInfo();
  const isBuild = command === "build";
  const strictI18n =
    process.env.OMNIGET_I18N_STRICT === "1" ||
    (isBuild && process.env.OMNIGET_I18N_STRICT !== "0");

  return {
    plugins: [
      devRevisionPlugin(configDir),
      previewDefaultsPlugin(configDir),
      i18nCheckPlugin({ strict: strictI18n }),
      pdfjsRuntimePlugin(),
      sveltekit({
        preprocess: vitePreprocess(),
        adapter: adapter({
          fallback: "index.html",
        }),
        alias: {
          "$lib": "src/lib",
          "$components": "src/components",
        },
        version: {
          pollInterval: 0,
        },
      }),
    ],

    define: {
      __COMMIT_HASH__: JSON.stringify(gitInfo.hash),
      __GIT_BRANCH__: JSON.stringify(gitInfo.branch),
      __APP_VERSION__: JSON.stringify(getVersion()),
      __BUILD_DATE__: JSON.stringify(new Date().toISOString().split("T")[0]),
    },

    // WHY: macOS Big Sur WebKit is the oldest engine we ship on (minimumSystemVersion 10.15); newer vite defaults would raise this.
    build: {
      target: ["es2020", "safari14"],
      cssTarget: ["safari14"],
    },

    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
