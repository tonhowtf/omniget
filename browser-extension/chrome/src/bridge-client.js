// Localhost HTTP bridge client.
//
// Replaces native messaging: instead of relying on Chrome's native-messaging
// manifest (which requires hard-coded extension IDs in the desktop app), we
// POST the same payload to the OmniGet desktop app's local HTTP listener,
// authenticating each request with a bearer token the user pastes once into
// the extension's options page.
//
// Lookup contract:
//   - The user copies `endpoint` (e.g. http://127.0.0.1:47720) and `token`
//     from the OmniGet Settings UI into the extension's Options page.
//   - chrome.storage.local stores them under the keys below.
//   - If either value is missing the bridge is considered "unpaired" and the
//     caller falls through to the omniget:// scheme handler.

export const STORAGE_KEY_ENDPOINT = "bridge_endpoint";
export const STORAGE_KEY_TOKEN = "bridge_token";
// Name of the low-frequency autopair alarm owned by background.js. Exported
// so the 401 recovery path below can re-arm it without duplicating the name.
export const AUTOPAIR_ALARM_NAME = "omniget-autopair";
const HEALTH_TIMEOUT_MS = 1500;
const ENQUEUE_TIMEOUT_MS = 8000;
const PROTOCOL_VERSION = 1;
const DEFAULT_ENDPOINT = "http://127.0.0.1:47720";
// Mirrors `PORT_RANGE` in src-tauri/src/local_bridge.rs — kept narrow so the
// auto-discovery probe is fast.
export const DEFAULT_PORT_RANGE = [
  47720, 47721, 47722, 47723, 47724, 47725, 47726, 47727, 47728, 47729,
];

export function trimEndpoint(value) {
  if (typeof value !== "string") return "";
  let trimmed = value.trim();
  if (!trimmed) return "";
  while (trimmed.endsWith("/")) {
    trimmed = trimmed.slice(0, -1);
  }
  return trimmed;
}

export async function loadBridgeConfig({ storage = globalThis.chrome?.storage?.local } = {}) {
  if (!storage?.get) {
    return { endpoint: "", token: "" };
  }
  return new Promise((resolve) => {
    storage.get([STORAGE_KEY_ENDPOINT, STORAGE_KEY_TOKEN], (items) => {
      const endpoint = trimEndpoint(items?.[STORAGE_KEY_ENDPOINT]) || DEFAULT_ENDPOINT;
      const token = typeof items?.[STORAGE_KEY_TOKEN] === "string" ? items[STORAGE_KEY_TOKEN].trim() : "";
      resolve({ endpoint, token });
    });
  });
}

export async function saveBridgeConfig(
  { endpoint = "", token = "" },
  { storage = globalThis.chrome?.storage?.local } = {}
) {
  if (!storage?.set) return false;
  return new Promise((resolve) => {
    storage.set(
      {
        [STORAGE_KEY_ENDPOINT]: trimEndpoint(endpoint),
        [STORAGE_KEY_TOKEN]: typeof token === "string" ? token.trim() : "",
      },
      () => resolve(true)
    );
  });
}

// Forget the stored token (keep the endpoint — it's still valid). Used when
// the desktop app rejects our bearer, e.g. after the user rotated the token
// in Settings; keeping the stale token around would leave this browser
// permanently unable to re-pair.
export async function clearStoredToken({ storage = globalThis.chrome?.storage?.local } = {}) {
  if (!storage?.set) return false;
  return new Promise((resolve) => {
    storage.set({ [STORAGE_KEY_TOKEN]: "" }, () => resolve(true));
  });
}

// 401/403 from the bridge means our token is stale (rotated or reset on the
// desktop side). Clear it and re-arm the autopair alarm so the next download
// fetches a fresh token on its own.
async function handleUnauthorized(storage) {
  try {
    await clearStoredToken({ storage });
  } catch {}
  try {
    const alarms = globalThis.chrome?.alarms;
    if (alarms?.create) {
      const existing = alarms.get ? await alarms.get(AUTOPAIR_ALARM_NAME) : null;
      if (!existing) {
        alarms.create(AUTOPAIR_ALARM_NAME, { periodInMinutes: 1 });
      }
    }
  } catch {}
}

function withTimeout(promise, ms, controller) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      try {
        controller?.abort();
      } catch {}
      reject(new Error(`bridge timed out after ${ms}ms`));
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}

// Probes the bridge port range in parallel and returns the first endpoint
// that answers `/v1/health` with `ok: true`. Used by the options page to
// pre-fill the URL without making the user discover the port by hand.
export async function discoverBridgeEndpoint({
  fetchImpl = typeof fetch !== "undefined" ? fetch : null,
  ports = DEFAULT_PORT_RANGE,
  hosts = ["127.0.0.1", "localhost"],
  host = null,
  timeoutMs = HEALTH_TIMEOUT_MS,
} = {}) {
  if (!fetchImpl) return null;
  const probeHosts = host ? [host] : hosts;
  const probes = probeHosts.flatMap((probeHost) =>
    ports.map(async (port) => {
      const endpoint = `http://${probeHost}:${port}`;
      const result = await checkBridgeHealth(endpoint, { fetchImpl, timeoutMs });
      return result.ok ? { endpoint, version: result.version ?? null } : null;
    })
  );

  const settled = await Promise.allSettled(probes);
  for (const result of settled) {
    if (result.status === "fulfilled" && result.value) {
      return result.value;
    }
  }
  return null;
}

export async function checkBridgeHealth(
  endpoint,
  { fetchImpl = (typeof fetch !== "undefined" ? fetch : null), timeoutMs = HEALTH_TIMEOUT_MS } = {}
) {
  if (!fetchImpl) return { ok: false, reason: "no-fetch" };
  const target = trimEndpoint(endpoint);
  if (!target) return { ok: false, reason: "no-endpoint" };

  const controller = typeof AbortController !== "undefined" ? new AbortController() : null;
  try {
    const response = await withTimeout(
      fetchImpl(`${target}/v1/health`, {
        method: "GET",
        signal: controller?.signal,
      }),
      timeoutMs,
      controller
    );
    if (!response.ok) {
      return { ok: false, reason: "http-error", status: response.status };
    }
    const body = await response.json().catch(() => null);
    return { ok: Boolean(body?.ok), version: body?.version ?? null };
  } catch (error) {
    return { ok: false, reason: "fetch-failed", message: error?.message ?? String(error) };
  }
}

function sleepMs(ms, sleep) {
  if (typeof sleep === "function") return sleep(ms);
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// If the desktop app is sitting in the tray (or not started), poke it awake
// through the omniget:// scheme and wait until `/v1/health` answers. The GUI
// is not required — the localhost bridge is the download backend.
export async function ensureBridgeReady({
  fetchImpl = typeof fetch !== "undefined" ? fetch : null,
  storage = globalThis.chrome?.storage?.local,
  openScheme,
  sleep,
  attempts = 40,
  intervalMs = 400,
} = {}) {
  const ping = async () => {
    const discovered = await discoverBridgeEndpoint({ fetchImpl });
    if (!discovered?.endpoint) return false;
    const current = await loadBridgeConfig({ storage });
    if (trimEndpoint(current.endpoint) !== discovered.endpoint) {
      await saveBridgeConfig(
        { endpoint: discovered.endpoint, token: current.token },
        { storage }
      );
    }
    return true;
  };

  if (await ping()) return { ok: true, woke: false };

  if (typeof openScheme === "function") {
    try {
      await openScheme("omniget://__wake");
    } catch {
      // Scheme handler missing or tabs API unavailable — keep polling anyway
      // in case the app is about to come up on its own.
    }
  }

  for (let i = 0; i < attempts; i++) {
    await sleepMs(intervalMs, sleep);
    if (await ping()) return { ok: true, woke: true };
  }
  return { ok: false, reason: "app-not-running" };
}

// One-shot auto-pair. If we don't have a token yet, discover the bridge and
// ask `GET /v1/pair`. The desktop app hands the token to this extension
// automatically — users never copy-paste or open Settings.
export async function autoPair({
  fetchImpl = typeof fetch !== "undefined" ? fetch : null,
  storage = globalThis.chrome?.storage?.local,
  timeoutMs = HEALTH_TIMEOUT_MS,
} = {}) {
  if (!fetchImpl) return { ok: false, reason: "no-fetch" };
  const current = await loadBridgeConfig({ storage });
  if (current.token) return { ok: true, reason: "already-paired" };

  let endpoint = current.endpoint;
  const discovered = await discoverBridgeEndpoint({ fetchImpl, timeoutMs });
  if (discovered?.endpoint) endpoint = discovered.endpoint;
  endpoint = trimEndpoint(endpoint);
  if (!endpoint) return { ok: false, reason: "no-endpoint" };

  const controller = typeof AbortController !== "undefined" ? new AbortController() : null;
  try {
    const response = await withTimeout(
      fetchImpl(`${endpoint}/v1/pair`, { method: "GET", signal: controller?.signal }),
      timeoutMs,
      controller
    );
    if (!response.ok) return { ok: false, reason: "window-closed" };
    const parsed = await response.json().catch(() => null);
    const token = typeof parsed?.token === "string" ? parsed.token.trim() : "";
    if (!parsed?.ok || !token) return { ok: false, reason: "no-token" };
    await saveBridgeConfig({ endpoint, token }, { storage });
    return { ok: true, reason: "paired", endpoint };
  } catch (error) {
    return { ok: false, reason: "error", message: error?.message ?? String(error) };
  }
}

export async function sendViaBridge(
  payload,
  {
    fetchImpl = (typeof fetch !== "undefined" ? fetch : null),
    storage = globalThis.chrome?.storage?.local,
    timeoutMs = ENQUEUE_TIMEOUT_MS,
    config = null,
  } = {}
) {
  if (!fetchImpl) {
    return { ok: false, reason: "no-fetch" };
  }

  const resolved = config ?? (await loadBridgeConfig({ storage }));
  const endpoint = trimEndpoint(resolved.endpoint);
  const token = typeof resolved.token === "string" ? resolved.token.trim() : "";

  if (!endpoint) {
    return { ok: false, reason: "missing-endpoint" };
  }
  if (!token) {
    return { ok: false, reason: "missing-token" };
  }

  const body = { ...payload, protocolVersion: PROTOCOL_VERSION };

  const controller = typeof AbortController !== "undefined" ? new AbortController() : null;
  let response;
  try {
    response = await withTimeout(
      fetchImpl(`${endpoint}/v1/enqueue`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify(body),
        signal: controller?.signal,
      }),
      timeoutMs,
      controller
    );
  } catch (error) {
    return {
      ok: false,
      reason: "fetch-failed",
      message: error?.message ?? String(error),
    };
  }

  let parsed = null;
  try {
    parsed = await response.json();
  } catch {
    parsed = null;
  }

  if (response.status === 401 || response.status === 403) {
    // Stale token (rotated/reset in the app) — drop it and re-arm autopair
    // so this browser can pair again instead of being locked out forever.
    await handleUnauthorized(storage);
    return {
      ok: false,
      reason: "unauthorized",
      status: response.status,
      message: parsed?.message ?? "Bridge rejected the bearer token",
    };
  }
  if (!response.ok) {
    return {
      ok: false,
      reason: "http-error",
      status: response.status,
      code: parsed?.code ?? null,
      message: parsed?.message ?? `HTTP ${response.status}`,
    };
  }

  return {
    ok: Boolean(parsed?.ok ?? false),
    code: parsed?.code ?? null,
    message: parsed?.message ?? null,
  };
}

// Probe a URL for its available resolutions/formats via the desktop app's
// `/v1/formats` endpoint. Returns `{ ok, title, qualities, ... }` on success,
// or `{ ok: false, reason, message }` mirroring `sendViaBridge`'s error shape
// so the in-page picker can show a helpful message.
export async function getFormatsViaBridge(
  payload,
  {
    fetchImpl = typeof fetch !== "undefined" ? fetch : null,
    storage = globalThis.chrome?.storage?.local,
    timeoutMs = 20000,
    config = null,
  } = {}
) {
  if (!fetchImpl) {
    return { ok: false, reason: "no-fetch" };
  }

  const resolved = config ?? (await loadBridgeConfig({ storage }));
  const endpoint = trimEndpoint(resolved.endpoint);
  const token = typeof resolved.token === "string" ? resolved.token.trim() : "";

  if (!endpoint) return { ok: false, reason: "missing-endpoint" };
  if (!token) return { ok: false, reason: "missing-token" };

  const body = { ...payload, protocolVersion: PROTOCOL_VERSION };

  const controller =
    typeof AbortController !== "undefined" ? new AbortController() : null;
  let response;
  try {
    response = await withTimeout(
      fetchImpl(`${endpoint}/v1/formats`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify(body),
        signal: controller?.signal,
      }),
      timeoutMs,
      controller
    );
  } catch (error) {
    return {
      ok: false,
      reason: "fetch-failed",
      message: error?.message ?? String(error),
    };
  }

  let parsed = null;
  try {
    parsed = await response.json();
  } catch {
    parsed = null;
  }

  if (response.status === 401 || response.status === 403) {
    await handleUnauthorized(storage);
    return {
      ok: false,
      reason: "unauthorized",
      status: response.status,
      message: parsed?.message ?? "Bridge rejected the bearer token",
    };
  }
  if (!response.ok) {
    return {
      ok: false,
      reason: "http-error",
      status: response.status,
      code: parsed?.code ?? null,
      message: parsed?.message ?? `HTTP ${response.status}`,
    };
  }

  return {
    ok: Boolean(parsed?.ok ?? false),
    title: parsed?.title ?? null,
    thumbnail: parsed?.thumbnail ?? null,
    mediaType: parsed?.mediaType ?? null,
    durationSeconds: parsed?.durationSeconds ?? null,
    qualities: Array.isArray(parsed?.qualities) ? parsed.qualities : [],
    message: parsed?.message ?? null,
  };
}

export async function sendCookiesViaBridge(
  cookies,
  {
    fetchImpl = (typeof fetch !== "undefined" ? fetch : null),
    storage = globalThis.chrome?.storage?.local,
    timeoutMs = ENQUEUE_TIMEOUT_MS,
    config = null,
    sourceUrl = null,
    pageTitle = null,
    alias = null,
  } = {}
) {
  if (!fetchImpl) {
    return { ok: false, reason: "no-fetch" };
  }

  const resolved = config ?? (await loadBridgeConfig({ storage }));
  const endpoint = trimEndpoint(resolved.endpoint);
  const token = typeof resolved.token === "string" ? resolved.token.trim() : "";

  if (!endpoint) {
    return { ok: false, reason: "missing-endpoint" };
  }
  if (!token) {
    return { ok: false, reason: "missing-token" };
  }

  const body = { cookies, protocolVersion: PROTOCOL_VERSION };
  if (sourceUrl) body.sourceUrl = sourceUrl;
  if (pageTitle) body.pageTitle = pageTitle;
  if (alias) body.alias = alias;

  const controller = typeof AbortController !== "undefined" ? new AbortController() : null;
  let response;
  try {
    response = await withTimeout(
      fetchImpl(`${endpoint}/v1/cookies`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify(body),
        signal: controller?.signal,
      }),
      timeoutMs,
      controller
    );
  } catch (error) {
    return {
      ok: false,
      reason: "fetch-failed",
      message: error?.message ?? String(error),
    };
  }

  let parsed = null;
  try {
    parsed = await response.json();
  } catch {
    parsed = null;
  }

  if (response.status === 401 || response.status === 403) {
    // Stale token (rotated/reset in the app) — drop it and re-arm autopair
    // so this browser can pair again instead of being locked out forever.
    await handleUnauthorized(storage);
    return {
      ok: false,
      reason: "unauthorized",
      status: response.status,
      message: parsed?.message ?? "Bridge rejected the bearer token",
    };
  }
  if (!response.ok) {
    return {
      ok: false,
      reason: "http-error",
      status: response.status,
      code: parsed?.code ?? null,
      message: parsed?.message ?? `HTTP ${response.status}`,
    };
  }

  return {
    ok: Boolean(parsed?.ok ?? false),
    code: parsed?.code ?? null,
    message: parsed?.message ?? null,
  };
}
