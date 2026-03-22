import test from "node:test";
import assert from "node:assert/strict";

import {
  SUCCESS_BADGE_BACKGROUND_COLOR,
  SUCCESS_BADGE_DURATION_MS,
  SUCCESS_BADGE_TEXT,
  createActionFeedbackController,
} from "../src/action-feedback.js";

function createFakeClock() {
  let nextId = 1;
  const timers = new Map();
  const cleared = [];

  return {
    cleared,
    clearTimeoutFn(timeoutId) {
      cleared.push(timeoutId);
      timers.delete(timeoutId);
    },
    getTimerIds() {
      return [...timers.keys()];
    },
    runTimer(timeoutId) {
      const timer = timers.get(timeoutId);
      assert.ok(timer, `timer ${timeoutId} should exist`);
      timers.delete(timeoutId);
      timer.callback();
    },
    setTimeoutFn(callback, delay) {
      const timeoutId = nextId;
      nextId += 1;
      timers.set(timeoutId, { callback, delay });
      return timeoutId;
    },
  };
}

test("shows a success badge and clears it after the default delay", async () => {
  const calls = [];
  const clock = createFakeClock();
  const controller = createActionFeedbackController({
    setBadgeText: async (details) => {
      calls.push(["text", details]);
    },
    setBadgeBackgroundColor: async (details) => {
      calls.push(["background", details]);
    },
    setTimeoutFn: clock.setTimeoutFn,
    clearTimeoutFn: clock.clearTimeoutFn,
  });

  await controller.showSuccessBadge(9);

  assert.deepEqual(calls, [
    ["background", { tabId: 9, color: SUCCESS_BADGE_BACKGROUND_COLOR }],
    ["text", { tabId: 9, text: SUCCESS_BADGE_TEXT }],
  ]);

  const [timeoutId] = clock.getTimerIds();
  assert.equal(clock.getTimerIds().length, 1);
  clock.runTimer(timeoutId);

  assert.deepEqual(calls.at(-1), ["text", { tabId: 9, text: "" }]);
});

test("resets the previous timer when a second success badge is shown for the same tab", async () => {
  const calls = [];
  const clock = createFakeClock();
  const controller = createActionFeedbackController({
    setBadgeText: async (details) => {
      calls.push(["text", details]);
    },
    setBadgeBackgroundColor: async (details) => {
      calls.push(["background", details]);
    },
    setTimeoutFn: clock.setTimeoutFn,
    clearTimeoutFn: clock.clearTimeoutFn,
    durationMs: SUCCESS_BADGE_DURATION_MS,
  });

  await controller.showSuccessBadge(4);
  const [firstTimeoutId] = clock.getTimerIds();

  await controller.showSuccessBadge(4);
  const [secondTimeoutId] = clock.getTimerIds();

  assert.deepEqual(clock.cleared, [firstTimeoutId]);
  assert.notEqual(secondTimeoutId, firstTimeoutId);
  assert.equal(clock.getTimerIds().length, 1);

  clock.runTimer(secondTimeoutId);

  const clearCalls = calls.filter(
    ([type, details]) => type === "text" && details.tabId === 4 && details.text === ""
  );
  assert.equal(clearCalls.length, 1);
});

test("clears any existing timer when the badge is explicitly cleared", async () => {
  const calls = [];
  const clock = createFakeClock();
  const controller = createActionFeedbackController({
    setBadgeText: async (details) => {
      calls.push(details);
    },
    setTimeoutFn: clock.setTimeoutFn,
    clearTimeoutFn: clock.clearTimeoutFn,
  });

  await controller.showSuccessBadge(2);
  const [timeoutId] = clock.getTimerIds();

  await controller.clearBadge(2);

  assert.deepEqual(clock.cleared, [timeoutId]);
  assert.deepEqual(calls.at(-1), { tabId: 2, text: "" });
  assert.equal(clock.getTimerIds().length, 0);
});
