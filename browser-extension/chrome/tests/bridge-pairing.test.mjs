import test from "node:test";
import assert from "node:assert/strict";
import * as bridge from "../src/bridge-client.js";

function memoryStorage(initial = {}) {
  const data = { ...initial };
  return {
    data,
    get(keys, cb) {
      const out = {};
      for (const k of keys) out[k] = data[k];
      cb(out);
    },
    set(items, cb) {
      Object.assign(data, items);
      cb?.();
    },
  };
}

function json(status, body) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

test("there is no pairing that discovers its own endpoint", () => {
  assert.equal(bridge.autoPair, undefined);
  assert.equal(bridge.AUTOPAIR_ALARM_NAME, undefined);
});

test("pairing asks only the endpoint the user confirmed", async () => {
  const storage = memoryStorage();
  const calls = [];
  const fetchImpl = async (url) => {
    calls.push(url);
    return json(200, { ok: true, token: " tok " });
  };
  const result = await bridge.pairWithEndpoint("http://127.0.0.1:47723/", { fetchImpl, storage });
  assert.deepEqual(calls, ["http://127.0.0.1:47723/v1/pair"]);
  assert.equal(result.ok, true);
  assert.equal(storage.data.bridge_endpoint, "http://127.0.0.1:47723");
  assert.equal(storage.data.bridge_token, "tok");
});

test("pairing without an endpoint asks nobody", async () => {
  const storage = memoryStorage();
  const calls = [];
  const fetchImpl = async (url) => {
    calls.push(url);
    return json(200, { ok: true, token: "tok" });
  };
  const result = await bridge.pairWithEndpoint("", { fetchImpl, storage });
  assert.deepEqual(calls, []);
  assert.deepEqual(result, { ok: false, reason: "no-endpoint" });
  assert.equal(storage.data.bridge_token, undefined);
});

test("a closed pairing window stores nothing", async () => {
  const storage = memoryStorage();
  const fetchImpl = async () => json(403, { ok: false });
  const result = await bridge.pairWithEndpoint("http://127.0.0.1:47720", { fetchImpl, storage });
  assert.deepEqual(result, { ok: false, reason: "window-closed" });
  assert.equal(storage.data.bridge_token, undefined);
});

test("a rejected token is cleared without scheduling a background pairing", async () => {
  const storage = memoryStorage({ bridge_endpoint: "http://127.0.0.1:47720", bridge_token: "old" });
  const created = [];
  const previous = globalThis.chrome;
  globalThis.chrome = { alarms: { get: async () => null, create: (...a) => created.push(a) } };
  try {
    const fetchImpl = async () => json(401, { message: "no" });
    const result = await bridge.sendCookiesViaBridge([{ name: "a" }], { fetchImpl, storage });
    assert.equal(result.reason, "unauthorized");
  } finally {
    globalThis.chrome = previous;
  }
  assert.equal(storage.data.bridge_token, "");
  assert.deepEqual(created, []);
});

test("cookies are not sent before a pairing exists", async () => {
  const storage = memoryStorage();
  const calls = [];
  const fetchImpl = async (url) => {
    calls.push(url);
    return json(200, { ok: true });
  };
  const result = await bridge.sendCookiesViaBridge([{ name: "a" }], { fetchImpl, storage });
  assert.deepEqual(result, { ok: false, reason: "missing-token" });
  assert.deepEqual(calls, []);
});
