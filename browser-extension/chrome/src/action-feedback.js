export const SUCCESS_BADGE_TEXT = "✓";
export const SUCCESS_BADGE_DURATION_MS = 1500;
export const SUCCESS_BADGE_BACKGROUND_COLOR = "#57b96a";

export function createActionFeedbackController({
  setBadgeText,
  setBadgeBackgroundColor = async () => {},
  setTimeoutFn = setTimeout,
  clearTimeoutFn = clearTimeout,
  durationMs = SUCCESS_BADGE_DURATION_MS,
  successText = SUCCESS_BADGE_TEXT,
  successColor = SUCCESS_BADGE_BACKGROUND_COLOR,
}) {
  const badgeTimeouts = new Map();

  function clearBadgeTimer(tabId) {
    const timeoutId = badgeTimeouts.get(tabId);
    if (timeoutId === undefined) {
      return;
    }

    clearTimeoutFn(timeoutId);
    badgeTimeouts.delete(tabId);
  }

  async function clearBadge(tabId) {
    if (tabId === undefined || tabId === null) {
      return;
    }

    clearBadgeTimer(tabId);
    await setBadgeText({
      tabId,
      text: "",
    });
  }

  async function showSuccessBadge(tabId) {
    if (tabId === undefined || tabId === null) {
      return;
    }

    clearBadgeTimer(tabId);

    await Promise.all([
      setBadgeBackgroundColor({
        tabId,
        color: successColor,
      }),
      setBadgeText({
        tabId,
        text: successText,
      }),
    ]);

    const timeoutId = setTimeoutFn(() => {
      badgeTimeouts.delete(tabId);
      void setBadgeText({
        tabId,
        text: "",
      });
    }, durationMs);

    badgeTimeouts.set(tabId, timeoutId);
  }

  return {
    clearBadge,
    showSuccessBadge,
  };
}
