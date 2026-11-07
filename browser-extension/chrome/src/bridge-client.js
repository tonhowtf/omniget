export const STORAGE_KEY_ENDPOINT = "bridge_endpoint";
export const STORAGE_KEY_TOKEN = "bridge_token";
export const AUTOPAIR_ALARM_NAME = "omniget-autopair";
const HEALTH_TIMEOUT_MS = 1500;
const ENQUEUE_TIMEOUT_MS = 8000;
const PROTOCOL_VERSION = 1;
const DEFAULT_ENDPOINT = "http://127.0.0.1:47720";
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

export function bridgeFailureMessageKey(reason) {
  switch (reason) {
    case "missing-token":
      return "popup_ckerr_missing_token";
    case "missing-endpoint":
    case "no-endpoint":
      return "popup_ckerr_missing_endpoint";
    case "unauthorized":
      return "popup_ckerr_unauthorized";
    case "http-error":
      return "popup_bridge_refused";
    case "tabs-create-failed":
    case "tabs-api-unavailable":
      return "popup_link_handler_failed";
    default:
      return "popup_not_running";
  }
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

export async function clearStoredToken({ storage = globalThis.chrome?.storage?.local } = {}) {
  if (!storage?.set) return false;
  return new Promise((resolve) => {
    storage.set({ [STORAGE_KEY_TOKEN]: "" }, () => resolve(true));
  });
}

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

  const optedOut = parsed?.code === "PLATFORM_OPTED_OUT";
  if (response.status === 401 || (response.status === 403 && !optedOut)) {
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
