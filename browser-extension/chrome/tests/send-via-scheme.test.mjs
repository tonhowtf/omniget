import test from "node:test";
import assert from "node:assert/strict";

import { buildOmnigetSchemeUrl, openOmnigetScheme } from "../src/send-via-scheme.js";

test("buildOmnigetSchemeUrl strips http(s) prefix and prepends omniget://", () => {
  assert.equal(
    buildOmnigetSchemeUrl("https://www.youtube.com/watch?v=abc"),
    "omniget://www.youtube.com/watch?v=abc"
  );
  assert.equal(
    buildOmnigetSchemeUrl("http://example.com/v"),
    "omniget://example.com/v"
  );
});

test("buildOmnigetSchemeUrl preserves an already-omniget URL", () => {
  assert.equal(
    buildOmnigetSchemeUrl("omniget://example.com/v"),
    "omniget://example.com/v"
  );
});

test("buildOmnigetSchemeUrl wraps magnet/p2p URLs without an extra slash", () => {
  assert.equal(
    buildOmnigetSchemeUrl("magnet:?xt=urn:btih:abc"),
    "omniget:magnet:?xt=urn:btih:abc"
  );
  assert.equal(
    buildOmnigetSchemeUrl("p2p:foo"),
    "omniget:p2p:foo"
  );
});

test("buildOmnigetSchemeUrl returns null for empty / non-string input", () => {
  assert.equal(buildOmnigetSchemeUrl(""), null);
  assert.equal(buildOmnigetSchemeUrl("   "), null);
  assert.equal(buildOmnigetSchemeUrl(null), null);
  assert.equal(buildOmnigetSchemeUrl(undefined), null);
  assert.equal(buildOmnigetSchemeUrl(42), null);
});

test("openOmnigetScheme creates a hidden tab and schedules its removal", async () => {
  const events = [];
  const scheduled = [];

  const tabs = {
    create: async (options) => {
      events.push(["create", options]);
      return { id: 1234 };
    },
    remove: (id) => {
      events.push(["remove", id]);
      return Promise.resolve();
    },
  };

  const schedule = (fn, ms) => {
    scheduled.push({ fn, ms });
    return null;
  };

  const result = await openOmnigetScheme("https://example.com/v", {
    tabs,
    schedule,
    tabLifetimeMs: 500,
  });

  assert.deepEqual(result, { ok: true, schemeUrl: "omniget://example.com/v" });
  assert.deepEqual(events, [
    ["create", { url: "omniget://example.com/v", active: false }],
  ]);
  assert.equal(scheduled.length, 1);
  assert.equal(scheduled[0].ms, 500);

  // Run the scheduled cleanup: the throwaway tab should be removed.
  scheduled[0].fn();
  assert.deepEqual(events[1], ["remove", 1234]);
});

test("openOmnigetScheme does not navigate the user's active tab", async () => {
  let updateCalled = false;
  const tabs = {
    create: async () => ({ id: 99 }),
    update: async () => {
      updateCalled = true;
    },
    remove: () => Promise.resolve(),
  };

  const result = await openOmnigetScheme("https://example.com/v", {
    tabs,
    schedule: () => {},
  });

  assert.equal(result.ok, true);
  assert.equal(updateCalled, false);
});

test("openOmnigetScheme returns invalid-url for empty input", async () => {
  const result = await openOmnigetScheme("", {
    tabs: { create: async () => ({ id: 1 }), remove: () => Promise.resolve() },
    schedule: () => {},
  });
  assert.deepEqual(result, { ok: false, reason: "invalid-url" });
});

test("openOmnigetScheme returns tabs-api-unavailable when tabs.create is missing", async () => {
  const result = await openOmnigetScheme("https://example.com/v", {
    tabs: {},
    schedule: () => {},
  });
  assert.deepEqual(result, { ok: false, reason: "tabs-api-unavailable" });
});

test("openOmnigetScheme reports tabs-create-failed when tabs.create throws", async () => {
  const tabs = {
    create: async () => {
      throw new Error("permission denied");
    },
    remove: () => Promise.resolve(),
  };

  const result = await openOmnigetScheme("https://example.com/v", {
    tabs,
    schedule: () => {},
  });

  assert.deepEqual(result, {
    ok: false,
    reason: "tabs-create-failed",
    message: "permission denied",
  });
});

test("openOmnigetScheme tolerates a sync remove() that throws", async () => {
  const tabs = {
    create: async () => ({ id: 7 }),
    remove: () => {
      throw new Error("tab is gone");
    },
  };

  const calls = [];
  const schedule = (fn) => {
    calls.push(fn);
  };

  const result = await openOmnigetScheme("https://example.com/v", {
    tabs,
    schedule,
  });

  assert.equal(result.ok, true);
  assert.doesNotThrow(() => calls[0]());
});
