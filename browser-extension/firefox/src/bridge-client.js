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
  host = "127.0.0.1",
  timeoutMs = HEALTH_TIMEOUT_MS,
} = {}) {
  if (!fetchImpl) return null;
  const probes = ports.map(async (port) => {
    const endpoint = `http://${host}:${port}`;
    const result = await checkBridgeHealth(endpoint, { fetchImpl, timeoutMs });
    return result.ok ? { endpoint, version: result.version ?? null } : null;
  });

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

export async function sendCookiesViaBridge(
  cookies,
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

  const body = { cookies, protocolVersion: PROTOCOL_VERSION };

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
