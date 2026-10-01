import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { execSync, spawnSync } from "child_process";
import { readFileSync, existsSync } from "fs";
import path from "path";
import { fileURLToPath } from "url";

import { devRevisionPlugin } from './scripts/dev/vite-plugin.mjs';
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

// The /read reader (vendored foliate-js) loads pdf.js runtime files from
// /vendor/pdfjs/; copy them from node_modules before serving or building.
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
      i18nCheckPlugin({ strict: strictI18n }),
      pdfjsRuntimePlugin(),
      sveltekit(),
    ],

    define: {
      __COMMIT_HASH__: JSON.stringify(gitInfo.hash),
      __GIT_BRANCH__: JSON.stringify(gitInfo.branch),
      __APP_VERSION__: JSON.stringify(getVersion()),
      __BUILD_DATE__: JSON.stringify(new Date().toISOString().split("T")[0]),
    },

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
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
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
