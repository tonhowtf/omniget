import test from "node:test";
import assert from "node:assert/strict";

import {
  trimEndpoint,
  loadBridgeConfig,
  saveBridgeConfig,
  checkBridgeHealth,
  discoverBridgeEndpoint,
  sendViaBridge,
  STORAGE_KEY_ENDPOINT,
  STORAGE_KEY_TOKEN,
  DEFAULT_PORT_RANGE,
} from "../src/bridge-client.js";

function fakeStorage(initial = {}) {
  const data = { ...initial };
  return {
    data,
    get: (keys, cb) => {
      const result = {};
      for (const key of keys) result[key] = data[key];
      cb(result);
    },
    set: (patch, cb) => {
      Object.assign(data, patch);
      cb();
    },
  };
}

test("trimEndpoint strips whitespace and trailing slashes", () => {
  assert.equal(trimEndpoint("  http://127.0.0.1:47720/  "), "http://127.0.0.1:47720");
  assert.equal(trimEndpoint("http://x/"), "http://x");
  assert.equal(trimEndpoint("http://x///"), "http://x");
  assert.equal(trimEndpoint(null), "");
  assert.equal(trimEndpoint(undefined), "");
});

test("loadBridgeConfig returns defaults when storage is empty", async () => {
  const storage = fakeStorage();
  const result = await loadBridgeConfig({ storage });
  assert.equal(result.endpoint, "http://127.0.0.1:47720");
  assert.equal(result.token, "");
});

test("loadBridgeConfig respects stored values", async () => {
  const storage = fakeStorage({
    [STORAGE_KEY_ENDPOINT]: "http://127.0.0.1:9999",
    [STORAGE_KEY_TOKEN]: "abc",
  });
  const result = await loadBridgeConfig({ storage });
  assert.equal(result.endpoint, "http://127.0.0.1:9999");
  assert.equal(result.token, "abc");
});

test("saveBridgeConfig writes trimmed values", async () => {
  const storage = fakeStorage();
  await saveBridgeConfig({ endpoint: " http://127.0.0.1:1234/ ", token: "  tok " }, { storage });
  assert.equal(storage.data[STORAGE_KEY_ENDPOINT], "http://127.0.0.1:1234");
  assert.equal(storage.data[STORAGE_KEY_TOKEN], "tok");
});

test("checkBridgeHealth returns ok when /v1/health responds with ok=true", async () => {
  const fetchImpl = async (url) => {
    assert.equal(url, "http://127.0.0.1:47720/v1/health");
    return { ok: true, json: async () => ({ ok: true, version: "0.5.3" }) };
  };
  const result = await checkBridgeHealth("http://127.0.0.1:47720", { fetchImpl });
  assert.deepEqual(result, { ok: true, version: "0.5.3" });
});

test("checkBridgeHealth surfaces a clean failure when fetch rejects", async () => {
  const fetchImpl = async () => { throw new Error("boom"); };
  const result = await checkBridgeHealth("http://127.0.0.1:47720", { fetchImpl });
  assert.equal(result.ok, false);
  assert.equal(result.reason, "fetch-failed");
  assert.equal(result.message, "boom");
});

test("checkBridgeHealth refuses an empty endpoint", async () => {
  const fetchImpl = async () => { throw new Error("should not call"); };
  const result = await checkBridgeHealth("", { fetchImpl });
  assert.deepEqual(result, { ok: false, reason: "no-endpoint" });
});

test("sendViaBridge POSTs the payload with bearer auth and protocolVersion", async () => {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    return {
      ok: true,
      status: 200,
      json: async () => ({ ok: true }),
    };
  };

  const result = await sendViaBridge(
    { type: "enqueue", url: "https://example.com/v" },
    {
      fetchImpl,
      config: { endpoint: "http://127.0.0.1:47720", token: "tok" },
    }
  );

  assert.deepEqual(result, { ok: true, code: null, message: null });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "http://127.0.0.1:47720/v1/enqueue");
  assert.equal(calls[0].init.method, "POST");
  assert.equal(calls[0].init.headers["Content-Type"], "application/json");
  assert.equal(calls[0].init.headers.Authorization, "Bearer tok");
  const body = JSON.parse(calls[0].init.body);
  assert.equal(body.url, "https://example.com/v");
  assert.equal(body.type, "enqueue");
  assert.equal(body.protocolVersion, 1);
});

test("sendViaBridge reports missing-token when no token is set", async () => {
  const fetchImpl = async () => { throw new Error("should not call"); };
  const result = await sendViaBridge(
    { url: "https://example.com" },
    {
      fetchImpl,
      config: { endpoint: "http://127.0.0.1:47720", token: "" },
    }
  );
  assert.deepEqual(result, { ok: false, reason: "missing-token" });
});

test("sendViaBridge reports unauthorized on HTTP 401", async () => {
  const fetchImpl = async () => ({
    ok: false,
    status: 401,
    json: async () => ({ ok: false, code: "UNAUTHORIZED", message: "bad token" }),
  });
  const result = await sendViaBridge(
    { url: "https://example.com" },
    {
      fetchImpl,
      config: { endpoint: "http://127.0.0.1:47720", token: "bad" },
    }
  );
  assert.equal(result.ok, false);
  assert.equal(result.reason, "unauthorized");
  assert.equal(result.status, 401);
  assert.equal(result.message, "bad token");
});

test("sendViaBridge surfaces network error so the caller can fall back", async () => {
  const fetchImpl = async () => { throw new Error("ECONNREFUSED"); };
  const result = await sendViaBridge(
    { url: "https://example.com" },
    {
      fetchImpl,
      config: { endpoint: "http://127.0.0.1:47720", token: "tok" },
    }
  );
  assert.equal(result.ok, false);
  assert.equal(result.reason, "fetch-failed");
  assert.equal(result.message, "ECONNREFUSED");
});

test("DEFAULT_PORT_RANGE matches the Rust bridge's PORT_RANGE", () => {
  // Sanity: keep the JS probe range narrow and aligned with
  // src-tauri/src/local_bridge.rs (47720..47730 = 10 ports).
  assert.equal(DEFAULT_PORT_RANGE[0], 47720);
  assert.equal(DEFAULT_PORT_RANGE.at(-1), 47729);
  assert.equal(DEFAULT_PORT_RANGE.length, 10);
});

test("discoverBridgeEndpoint returns the first port that answers /v1/health", async () => {
  const seen = [];
  const fetchImpl = async (url) => {
    seen.push(url);
    if (url === "http://127.0.0.1:47722/v1/health") {
      return { ok: true, json: async () => ({ ok: true, version: "0.5.3" }) };
    }
    return { ok: false, json: async () => ({}) };
  };

  const result = await discoverBridgeEndpoint({
    fetchImpl,
    ports: [47720, 47721, 47722, 47723],
  });

  assert.deepEqual(result, {
    endpoint: "http://127.0.0.1:47722",
    version: "0.5.3",
  });
  // All four ports were probed in parallel.
  assert.equal(seen.length, 4);
});

test("discoverBridgeEndpoint returns null when no port responds", async () => {
  const fetchImpl = async () => { throw new Error("ECONNREFUSED"); };
  const result = await discoverBridgeEndpoint({
    fetchImpl,
    ports: [47720, 47721],
  });
  assert.equal(result, null);
});

test("discoverBridgeEndpoint ignores 200 responses where ok is false", async () => {
  const fetchImpl = async () => ({
    ok: true,
    json: async () => ({ ok: false }),
  });
  const result = await discoverBridgeEndpoint({
    fetchImpl,
    ports: [47720],
  });
  assert.equal(result, null);
});

test("sendViaBridge bubbles up app-level error code on a 4xx response", async () => {
  const fetchImpl = async () => ({
    ok: false,
    status: 400,
    json: async () => ({ ok: false, code: "INVALID_URL", message: "bad URL" }),
  });
  const result = await sendViaBridge(
    { url: "not-a-url" },
    {
      fetchImpl,
      config: { endpoint: "http://127.0.0.1:47720", token: "tok" },
    }
  );
  assert.equal(result.ok, false);
  assert.equal(result.reason, "http-error");
  assert.equal(result.status, 400);
  assert.equal(result.code, "INVALID_URL");
  assert.equal(result.message, "bad URL");
});
