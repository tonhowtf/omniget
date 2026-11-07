import test from "node:test";
import assert from "node:assert/strict";
import { bridgeFailureMessageKey } from "../src/bridge-client.js";

test("popup names a missing pairing instead of saying the app is not running", () => {
  assert.equal(bridgeFailureMessageKey("missing-token"), "popup_ckerr_missing_token");
});

test("popup names a rejected token", () => {
  assert.equal(bridgeFailureMessageKey("unauthorized"), "popup_ckerr_unauthorized");
});

test("popup says the app refused when the bridge answers with an error", () => {
  assert.equal(bridgeFailureMessageKey("http-error"), "popup_bridge_refused");
});

test("popup says the link handler failed when the browser cannot open it", () => {
  assert.equal(bridgeFailureMessageKey("tabs-create-failed"), "popup_link_handler_failed");
});

test("popup says the app is not running when nothing answers", () => {
  assert.equal(bridgeFailureMessageKey("fetch-failed"), "popup_not_running");
});
