import test from "node:test";
import assert from "node:assert/strict";

function makeChromeStub({ grant = true, storage = {}, hasContains = true, hasRequest = true } = {}) {
  const backing = { ...storage };
  const calls = { contains: 0, request: 0 };
  const chrome = {
    permissions: {},
    storage: {
      local: {
        get: async (key) => {
          if (typeof key === "string") {
            return key in backing ? { [key]: backing[key] } : {};
          }
          return { ...backing };
        },
        set: async (obj) => {
          Object.assign(backing, obj);
        },
      },
    },
    __backing: backing,
    __calls: calls,
  };
  if (hasContains) {
    chrome.permissions.contains = async () => {
      calls.contains += 1;
      return grant;
    };
  }
  if (hasRequest) {
    chrome.permissions.request = async () => {
      calls.request += 1;
      return grant;
    };
  }
  return chrome;
}

async function loadModuleFresh() {
  const url = new URL("../src/sniffer-toggle.js", import.meta.url);
  url.searchParams.set("_t", String(Math.random()));
  return await import(url.href);
}

test("enabling the sniffer requests wildcard permission and succeeds when granted", async () => {
  globalThis.chrome = makeChromeStub({ grant: true });
  const mod = await loadModuleFresh();

  const result = await mod.setSnifferEnabled(true);

  assert.deepEqual(result, { ok: true });
  assert.equal(globalThis.chrome.__calls.request, 1);
  assert.equal(globalThis.chrome.__backing.omniget_sniffer_enabled, true);
  assert.equal(mod.isSnifferEnabled(), true);
});

test("enabling the sniffer fails closed and does not persist when permission is denied", async () => {
  globalThis.chrome = makeChromeStub({ grant: false });
  const mod = await loadModuleFresh();

  const result = await mod.setSnifferEnabled(true);

  assert.equal(result.ok, false);
  assert.equal(result.reason, "permission_denied");
  assert.equal(globalThis.chrome.__backing.omniget_sniffer_enabled, false);
  assert.equal(mod.isSnifferEnabled(), false);
});

test("disabling the sniffer never requests permission", async () => {
  globalThis.chrome = makeChromeStub({ grant: true });
  const mod = await loadModuleFresh();

  const result = await mod.setSnifferEnabled(false);

  assert.deepEqual(result, { ok: true });
  assert.equal(globalThis.chrome.__calls.request, 0);
  assert.equal(globalThis.chrome.__backing.omniget_sniffer_enabled, false);
  assert.equal(mod.isSnifferEnabled(), false);
});

test("hasWildcardHostPermission returns true when chrome.permissions is unavailable", async () => {
  globalThis.chrome = { storage: { local: { get: async () => ({}), set: async () => {} } } };
  const mod = await loadModuleFresh();

  const has = await mod.hasWildcardHostPermission();

  assert.equal(has, true);
});

test("requestWildcardHostPermission returns true when chrome.permissions.request is unavailable", async () => {
  globalThis.chrome = makeChromeStub({ hasRequest: false });
  const mod = await loadModuleFresh();

  const granted = await mod.requestWildcardHostPermission();

  assert.equal(granted, true);
});

test("loadSnifferState defaults to enabled when no prior setting", async () => {
  globalThis.chrome = makeChromeStub({ storage: {} });
  const mod = await loadModuleFresh();

  const enabled = await mod.loadSnifferState();

  assert.equal(enabled, true);
});

test("loadSnifferState honours a previously-disabled flag", async () => {
  globalThis.chrome = makeChromeStub({ storage: { omniget_sniffer_enabled: false } });
  const mod = await loadModuleFresh();

  const enabled = await mod.loadSnifferState();

  assert.equal(enabled, false);
});

test("loadSnifferState answers the same promise to every caller", async () => {
  let reads = 0;
  globalThis.chrome = makeChromeStub({ storage: { omniget_sniffer_enabled: false } });
  const realGet = globalThis.chrome.storage.local.get;
  globalThis.chrome.storage.local.get = async (key) => {
    reads += 1;
    return realGet(key);
  };
  const mod = await loadModuleFresh();

  // The sniffer listeners consult this on every captured request; it must not
  // hit storage each time.
  const [a, b, c] = await Promise.all([
    mod.loadSnifferState(),
    mod.loadSnifferState(),
    mod.loadSnifferState(),
  ]);

  assert.equal(reads, 1);
  assert.deepEqual([a, b, c], [false, false, false]);
});

test("loadSnifferState reflects a toggle made after the first read", async () => {
  globalThis.chrome = makeChromeStub({ grant: true, storage: {} });
  const mod = await loadModuleFresh();

  assert.equal(await mod.loadSnifferState(), true);
  await mod.setSnifferEnabled(false);
  assert.equal(await mod.loadSnifferState(), false);
  await mod.setSnifferEnabled(true);
  assert.equal(await mod.loadSnifferState(), true);
});

test("loadSnifferState keeps the default when storage throws", async () => {
  globalThis.chrome = { storage: { local: { get: async () => { throw new Error("no storage"); } } } };
  const mod = await loadModuleFresh();

  assert.equal(await mod.loadSnifferState(), true);
});
